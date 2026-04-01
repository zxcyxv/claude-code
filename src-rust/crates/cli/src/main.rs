// claude-code CLI entry point
//
// This is the main binary for the Claude Code Rust port. It:
// 1. Parses CLI arguments with clap (mirrors cli.tsx + main.tsx flags)
// 2. Loads configuration from settings.json + env vars
// 3. Builds system/user context (git status, CLAUDE.md)
// 4. Runs in either:
//    - Headless (--print / -p) mode: single query, output to stdout
//    - Interactive REPL mode: full TUI with ratatui

mod oauth_flow;

use anyhow::Context;
use async_trait::async_trait;
use cc_core::types::ToolDefinition;
use cc_core::{
    config::{Config, PermissionMode, Settings},
    constants::{APP_VERSION, DEFAULT_MODEL},
    context::ContextBuilder,
    cost::CostTracker,
    permissions::{AutoPermissionHandler, InteractivePermissionHandler},
};
use cc_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use clap::{ArgAction, Parser, ValueEnum};
use std::{path::PathBuf, sync::Arc};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// MCP tool wrapper: makes MCP server tools look like native cc-tools.
// ---------------------------------------------------------------------------

struct McpToolWrapper {
    tool_def: ToolDefinition,
    server_name: String,
    manager: Arc<cc_mcp::McpManager>,
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.tool_def.name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn permission_level(&self) -> PermissionLevel {
        // MCP tools run external processes – treat as Execute.
        PermissionLevel::Execute
    }

    fn input_schema(&self) -> serde_json::Value {
        self.tool_def.input_schema.clone()
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // Strip the server-name prefix to get the bare tool name.
        let prefix = format!("{}_", self.server_name);
        let bare_name = self
            .tool_def
            .name
            .strip_prefix(&prefix)
            .unwrap_or(&self.tool_def.name);

        let args = if input.is_null() { None } else { Some(input) };

        match self.manager.call_tool(&self.tool_def.name, args).await {
            Ok(result) => {
                let text = cc_mcp::mcp_result_to_string(&result);
                if result.is_error {
                    ToolResult::error(text)
                } else {
                    ToolResult::success(text)
                }
            }
            Err(e) => ToolResult::error(format!("MCP tool '{}' failed: {}", bare_name, e)),
        }
    }
}

// ---------------------------------------------------------------------------
// CLI argument definition (matches TypeScript main.tsx flags)
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "claude",
    version = APP_VERSION,
    about = "Claude Code - AI-powered coding assistant",
    long_about = None,
)]
struct Cli {
    /// Initial prompt to send (enables headless/print mode)
    prompt: Option<String>,

    /// Print mode: send prompt and exit (non-interactive)
    #[arg(short = 'p', long = "print", action = ArgAction::SetTrue)]
    print: bool,

    /// Model to use
    #[arg(short = 'm', long = "model", default_value = DEFAULT_MODEL)]
    model: String,

    /// Permission mode
    #[arg(long = "permission-mode", value_enum, default_value_t = CliPermissionMode::Default)]
    permission_mode: CliPermissionMode,

    /// Resume a previous session by ID
    #[arg(long = "resume")]
    resume: Option<String>,

    /// Maximum number of agentic turns
    #[arg(long = "max-turns", default_value_t = 10)]
    max_turns: u32,

    /// Custom system prompt
    #[arg(long = "system-prompt", short = 's')]
    system_prompt: Option<String>,

    /// Append to system prompt
    #[arg(long = "append-system-prompt")]
    append_system_prompt: Option<String>,

    /// Disable CLAUDE.md memory files
    #[arg(long = "no-claude-md", action = ArgAction::SetTrue)]
    no_claude_md: bool,

    /// Output format
    #[arg(long = "output-format", value_enum, default_value_t = CliOutputFormat::Text)]
    output_format: CliOutputFormat,

    /// Enable verbose logging
    #[arg(long = "verbose", short = 'v', action = ArgAction::SetTrue)]
    verbose: bool,

    /// API key (overrides ANTHROPIC_API_KEY env var)
    #[arg(long = "api-key")]
    api_key: Option<String>,

    /// Maximum tokens per response
    #[arg(long = "max-tokens")]
    max_tokens: Option<u32>,

    /// Working directory
    #[arg(long = "cwd")]
    cwd: Option<PathBuf>,

    /// Bypass all permission checks (danger!)
    #[arg(long = "dangerously-skip-permissions", action = ArgAction::SetTrue)]
    dangerously_skip_permissions: bool,

    /// Dump the system prompt to stdout and exit
    #[arg(long = "dump-system-prompt", action = ArgAction::SetTrue, hide = true)]
    dump_system_prompt: bool,

    /// MCP config JSON string (inline server definitions)
    #[arg(long = "mcp-config")]
    mcp_config: Option<String>,

    /// Disable auto-compaction
    #[arg(long = "no-auto-compact", action = ArgAction::SetTrue)]
    no_auto_compact: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum CliPermissionMode {
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
}

impl From<CliPermissionMode> for PermissionMode {
    fn from(m: CliPermissionMode) -> Self {
        match m {
            CliPermissionMode::Default => PermissionMode::Default,
            CliPermissionMode::AcceptEdits => PermissionMode::AcceptEdits,
            CliPermissionMode::BypassPermissions => PermissionMode::BypassPermissions,
            CliPermissionMode::Plan => PermissionMode::Plan,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum CliOutputFormat {
    Text,
    Json,
    #[value(name = "stream-json")]
    StreamJson,
}

impl From<CliOutputFormat> for cc_core::config::OutputFormat {
    fn from(f: CliOutputFormat) -> Self {
        match f {
            CliOutputFormat::Text => cc_core::config::OutputFormat::Text,
            CliOutputFormat::Json => cc_core::config::OutputFormat::Json,
            CliOutputFormat::StreamJson => cc_core::config::OutputFormat::StreamJson,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Fast-path: handle --version before parsing everything
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.iter().any(|a| a == "--version" || a == "-V") {
        println!("claude {}", APP_VERSION);
        return Ok(());
    }

    // Fast-path: `claude auth <login|logout|status>` — mirrors TypeScript cli.tsx pattern
    if raw_args.get(1).map(|s| s.as_str()) == Some("auth") {
        return handle_auth_command(&raw_args[2..]).await;
    }

    // Fast-path: named commands (`claude agents`, `claude ide`, `claude branch`, …)
    // Check before Cli::parse() so these names don't conflict with positional prompt arg.
    if let Some(cmd_name) = raw_args.get(1).map(|s| s.as_str()) {
        // Only intercept if it looks like a subcommand (no leading `-` or `/`)
        if !cmd_name.starts_with('-') && !cmd_name.starts_with('/') {
            if let Some(named_cmd) = cc_commands::named_commands::find_named_command(cmd_name) {
                // Build a minimal CommandContext (named commands are pre-session)
                let settings = Settings::load().await.unwrap_or_default();
                let config = settings.config.clone();
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let cmd_ctx = cc_commands::CommandContext {
                    config,
                    cost_tracker: CostTracker::new(),
                    messages: vec![],
                    working_dir: cwd,
                };
                // Collect remaining args after the command name
                let rest: Vec<&str> = raw_args[2..].iter().map(|s| s.as_str()).collect();
                let result = named_cmd.execute_named(&rest, &cmd_ctx);
                match result {
                    cc_commands::CommandResult::Message(msg)
                    | cc_commands::CommandResult::UserMessage(msg) => {
                        println!("{}", msg);
                        std::process::exit(0);
                    }
                    cc_commands::CommandResult::Error(e) => {
                        eprintln!("Error: {}", e);
                        eprintln!("Usage: {}", named_cmd.usage());
                        std::process::exit(1);
                    }
                    _ => {
                        // For any other result variant, fall through to normal startup
                    }
                }
                return Ok(());
            }
        }
    }

    let cli = Cli::parse();

    // Setup logging
    let log_level = if cli.verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .with_target(false)
        .without_time()
        .init();

    // Determine working directory
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    debug!(cwd = %cwd.display(), "Starting Claude Code");

    // Load settings from disk
    let settings = Settings::load().await.unwrap_or_default();

    // Build effective config (CLI args override settings)
    let mut config = settings.config.clone();
    if let Some(ref key) = cli.api_key {
        config.api_key = Some(key.clone());
    }
    config.model = Some(cli.model.clone());
    if let Some(mt) = cli.max_tokens {
        config.max_tokens = Some(mt);
    }
    config.verbose = cli.verbose;
    config.output_format = cli.output_format.into();
    config.disable_claude_mds = cli.no_claude_md;
    if let Some(sp) = cli.system_prompt.clone() {
        config.custom_system_prompt = Some(sp);
    }
    if let Some(asp) = cli.append_system_prompt.clone() {
        config.append_system_prompt = Some(asp);
    }
    if cli.dangerously_skip_permissions {
        config.permission_mode = PermissionMode::BypassPermissions;
    } else {
        config.permission_mode = cli.permission_mode.into();
    }
    if cli.no_auto_compact {
        config.auto_compact = false;
    }
    config.project_dir = Some(cwd.clone());

    // --dump-system-prompt fast path
    if cli.dump_system_prompt {
        let ctx = ContextBuilder::new(cwd.clone()).disable_claude_mds(config.disable_claude_mds);
        let sys = ctx.build_system_context().await;
        let user = ctx.build_user_context().await;
        println!("{}\n\n{}", sys, user);
        return Ok(());
    }

    // Build context
    let ctx_builder =
        ContextBuilder::new(cwd.clone()).disable_claude_mds(config.disable_claude_mds);
    let system_ctx = ctx_builder.build_system_context().await;
    let user_ctx = ctx_builder.build_user_context().await;

    // Build system prompt
    let mut system_parts = vec![
        include_str!("system_prompt.txt").to_string(),
        system_ctx,
        user_ctx,
    ];
    if let Some(ref custom) = config.custom_system_prompt {
        // replace base system prompt
        system_parts[0] = custom.clone();
    }
    if let Some(ref append) = config.append_system_prompt {
        system_parts.push(append.clone());
    }
    let system_prompt = system_parts.join("\n\n");

    // Determine mode early (needed for auth error handling and permission handler selection).
    let is_headless = cli.print || cli.prompt.is_some();
    let use_local_backend = config.use_local_backend();

    // Initialize API client.
    // Try config/env first; fall back to saved OAuth tokens; finally prompt for login.
    let (api_key, use_bearer_auth) = match config.resolve_auth_async().await {
        Some(auth) => auth,
        None => {
            // No credential found — run interactive OAuth login (non-headless) or error.
            if use_local_backend {
                ("local".to_string(), false)
            } else {
                if is_headless {
                    anyhow::bail!(
                        "No API key found. Set ANTHROPIC_API_KEY, use --api-key, or run `claude login`."
                    );
                }
                eprintln!("No authentication found. Starting login flow...");
                let result = oauth_flow::run_oauth_login_flow(true)
                    .await
                    .context("Login failed")?;
                println!("Login successful!");
                (result.credential, result.use_bearer_auth)
            }
        }
    };

    let client_config = cc_api::client::ClientConfig {
        api_key,
        api_base: config.resolve_api_base(),
        use_bearer_auth,
        ..Default::default()
    };
    let client = Arc::new(
        cc_api::AnthropicClient::new(client_config).context("Failed to create API client")?,
    );

    // Build tools
    // Interactive mode uses InteractivePermissionHandler which allows writes in Default mode
    // (the user is watching the TUI so they can intervene). Headless/print mode uses
    // AutoPermissionHandler which denies writes in Default mode for safety.
    let permission_handler: Arc<dyn cc_core::PermissionHandler> = if is_headless {
        Arc::new(AutoPermissionHandler {
            mode: config.permission_mode.clone(),
        })
    } else {
        Arc::new(InteractivePermissionHandler {
            mode: config.permission_mode.clone(),
        })
    };
    let cost_tracker = CostTracker::new();
    let session_id = uuid::Uuid::new_v4().to_string();

    // Initialize MCP servers first (needed for ToolContext.mcp_manager).
    let mcp_manager_arc: Option<Arc<cc_mcp::McpManager>> = if !config.mcp_servers.is_empty() {
        info!(
            count = config.mcp_servers.len(),
            "Connecting to MCP servers"
        );
        let mcp_manager = cc_mcp::McpManager::connect_all(&config.mcp_servers).await;
        if mcp_manager.server_count() > 0 {
            Some(Arc::new(mcp_manager))
        } else {
            None
        }
    } else {
        None
    };

    let tool_ctx = ToolContext {
        working_dir: cwd.clone(),
        permission_mode: config.permission_mode.clone(),
        permission_handler: permission_handler.clone(),
        cost_tracker: cost_tracker.clone(),
        session_id: session_id.clone(),
        non_interactive: cli.print || cli.prompt.is_some(),
        mcp_manager: mcp_manager_arc.clone(),
        config: config.clone(),
    };

    // Build the full tool list: built-ins from cc-tools plus AgentTool from cc-query
    // (AgentTool lives in cc-query to avoid a circular cc-tools ↔ cc-query dependency).
    // Wrap in Arc so the list can be shared by the main loop AND the cron scheduler.
    let tools: Arc<Vec<Box<dyn cc_tools::Tool>>> = {
        let mut v: Vec<Box<dyn cc_tools::Tool>> = cc_tools::all_tools();
        v.push(Box::new(cc_query::AgentTool));

        // Register MCP server tools as wrappers.
        if let Some(ref manager_arc) = mcp_manager_arc {
            for (server_name, tool_def) in manager_arc.all_tool_definitions() {
                let wrapper = McpToolWrapper {
                    tool_def,
                    server_name,
                    manager: manager_arc.clone(),
                };
                v.push(Box::new(wrapper));
            }
            debug!(total_tools = v.len(), "MCP tools registered");
        }

        Arc::new(v)
    };

    // Build query config
    let query_config = cc_query::QueryConfig {
        model: config.effective_model().to_string(),
        max_tokens: config.effective_max_tokens(),
        max_turns: cli.max_turns,
        system_prompt: Some(system_prompt),
        append_system_prompt: None,
        output_style: config.effective_output_style(),
        working_directory: Some(cwd.display().to_string()),
        thinking_budget: None,
        temperature: None,
    };

    // Spawn the background cron scheduler (fires cron tasks at scheduled times).
    // Cancelled automatically when the process exits since we use a shared token.
    let cron_cancel = tokio_util::sync::CancellationToken::new();
    cc_query::start_cron_scheduler(
        client.clone(),
        tools.clone(),
        tool_ctx.clone(),
        query_config.clone(),
        cron_cancel.clone(),
    );

    // --print mode (headless)
    let result = if is_headless {
        run_headless(&cli, client, tools, tool_ctx, query_config, cost_tracker).await
    } else {
        run_interactive(
            config,
            client,
            tools,
            tool_ctx,
            query_config,
            cost_tracker,
            cli.resume,
        )
        .await
    };

    cron_cancel.cancel();
    result
}

// ---------------------------------------------------------------------------
// Headless mode: read prompt from arg/stdin, run, print response
// ---------------------------------------------------------------------------

async fn run_headless(
    cli: &Cli,
    client: Arc<cc_api::AnthropicClient>,
    tools: Arc<Vec<Box<dyn cc_tools::Tool>>>,
    tool_ctx: ToolContext,
    query_config: cc_query::QueryConfig,
    cost_tracker: Arc<CostTracker>,
) -> anyhow::Result<()> {
    use cc_query::{QueryEvent, QueryOutcome};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    // Read prompt from positional arg or stdin
    let prompt = if let Some(ref p) = cli.prompt {
        p.clone()
    } else {
        // Read from stdin
        use tokio::io::{self, AsyncReadExt};
        let mut stdin = io::stdin();
        let mut buf = String::new();
        stdin.read_to_string(&mut buf).await?;
        buf.trim().to_string()
    };

    if prompt.is_empty() {
        eprintln!("Error: No prompt provided. Use --print <prompt> or pipe text to stdin.");
        std::process::exit(1);
    }

    let is_json_output = matches!(
        cli.output_format,
        CliOutputFormat::Json | CliOutputFormat::StreamJson
    );
    let is_stream_json = matches!(cli.output_format, CliOutputFormat::StreamJson);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<QueryEvent>();
    let cancel = CancellationToken::new();

    // Spawn the query loop in a background task so we can drain events concurrently
    let mut messages = vec![cc_core::types::Message::user(prompt)];
    let client_clone = client.clone();
    let tool_ctx_clone = tool_ctx.clone();
    let qcfg = query_config.clone();
    let tracker_clone = cost_tracker.clone();
    let event_tx_clone = event_tx.clone();
    let cancel_clone = cancel.clone();

    let query_handle = tokio::spawn(async move {
        cc_query::run_query_loop(
            client_clone.as_ref(),
            &mut messages,
            tools.as_slice(),
            &tool_ctx_clone,
            &qcfg,
            tracker_clone,
            Some(event_tx_clone),
            cancel_clone,
        )
        .await
    });

    // Drop the original tx so the channel closes when the task drops its clone
    drop(event_tx);

    // Drain events and print streaming text
    let mut full_text = String::new();

    while let Some(event) = event_rx.recv().await {
        match &event {
            QueryEvent::Stream(cc_api::StreamEvent::ContentBlockDelta {
                delta: cc_api::streaming::ContentDelta::TextDelta { text },
                ..
            }) => {
                full_text.push_str(text);
                if !is_json_output {
                    print!("{}", text);
                    use std::io::Write;
                    std::io::stdout().flush().ok();
                } else if is_stream_json {
                    let chunk = serde_json::json!({ "type": "text_delta", "text": text });
                    println!("{}", chunk);
                }
            }
            QueryEvent::ToolStart { tool_name, .. } => {
                if !is_json_output {
                    eprintln!("\n[{}...]", tool_name);
                } else {
                    let ev = serde_json::json!({ "type": "tool_start", "tool": tool_name });
                    println!("{}", ev);
                }
            }
            QueryEvent::Error(msg) => {
                if is_json_output {
                    let ev = serde_json::json!({ "type": "error", "error": msg });
                    eprintln!("{}", ev);
                } else {
                    eprintln!("\nError: {}", msg);
                }
            }
            _ => {}
        }
    }

    // Wait for the query task to finish and get the final outcome
    let outcome =
        query_handle
            .await
            .unwrap_or(QueryOutcome::Error(cc_core::error::ClaudeError::Other(
                "Query task panicked".to_string(),
            )));

    // Final output
    match cli.output_format {
        CliOutputFormat::Json => match outcome {
            QueryOutcome::EndTurn { message, usage } => {
                let result_text = if full_text.is_empty() {
                    message.get_all_text()
                } else {
                    full_text
                };
                let out = serde_json::json!({
                    "type": "result",
                    "result": result_text,
                    "usage": {
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens,
                        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
                        "cache_read_input_tokens": usage.cache_read_input_tokens,
                    },
                    "cost_usd": cost_tracker.total_cost_usd(),
                });
                println!("{}", out);
            }
            QueryOutcome::Error(e) => {
                let out = serde_json::json!({ "type": "error", "error": e.to_string() });
                eprintln!("{}", out);
                std::process::exit(1);
            }
            _ => {}
        },
        CliOutputFormat::StreamJson => {
            // Already streamed above; emit final result event
            match outcome {
                QueryOutcome::EndTurn { usage, .. } => {
                    let out = serde_json::json!({
                        "type": "result",
                        "usage": {
                            "input_tokens": usage.input_tokens,
                            "output_tokens": usage.output_tokens,
                        },
                        "cost_usd": cost_tracker.total_cost_usd(),
                    });
                    println!("{}", out);
                }
                QueryOutcome::Error(e) => {
                    let out = serde_json::json!({ "type": "error", "error": e.to_string() });
                    eprintln!("{}", out);
                    std::process::exit(1);
                }
                _ => {}
            }
        }
        CliOutputFormat::Text => {
            // Streaming text was already printed; add newline
            println!();
            if cli.verbose {
                eprintln!(
                    "\nTokens: {} in / {} out | Cost: ${:.4}",
                    cost_tracker.input_tokens(),
                    cost_tracker.output_tokens(),
                    cost_tracker.total_cost_usd(),
                );
            }
            if let QueryOutcome::Error(e) = outcome {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive REPL mode
// ---------------------------------------------------------------------------

async fn run_interactive(
    config: Config,
    client: Arc<cc_api::AnthropicClient>,
    tools: Arc<Vec<Box<dyn cc_tools::Tool>>>,
    tool_ctx: ToolContext,
    query_config: cc_query::QueryConfig,
    cost_tracker: Arc<CostTracker>,
    resume_id: Option<String>,
) -> anyhow::Result<()> {
    use cc_commands::{execute_command, CommandContext, CommandResult};
    use cc_query::{QueryEvent, QueryOutcome};
    use cc_tui::{render::render_app, restore_terminal, setup_terminal, App};
    use crossterm::event::{self, Event, KeyCode};
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    // Load previous session if resuming
    let initial_messages = if let Some(ref id) = resume_id {
        match cc_core::history::load_session(id).await {
            Ok(session) => {
                println!("Resumed session: {}", id);
                session.messages
            }
            Err(e) => {
                eprintln!("Warning: could not load session {}: {}", id, e);
                vec![]
            }
        }
    } else {
        vec![]
    };

    // Set up terminal
    let mut terminal = setup_terminal()?;
    let mut app = App::new(config.clone(), cost_tracker.clone());
    app.messages = initial_messages.clone();

    let mut messages = initial_messages;
    let mut cmd_ctx = CommandContext {
        config: config.clone(),
        cost_tracker: cost_tracker.clone(),
        messages: messages.clone(),
        working_dir: tool_ctx.working_dir.clone(),
    };

    // tools is already Arc<Vec<...>> — share it across spawned tasks without copying.
    let tools_arc = tools;

    // Current cancel token (replaced each turn)
    let mut cancel: Option<CancellationToken> = None;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<QueryEvent>();
    type MessagesArc = Arc<tokio::sync::Mutex<Vec<cc_core::types::Message>>>;
    let mut current_query: Option<(tokio::task::JoinHandle<QueryOutcome>, MessagesArc)> = None;

    'main: loop {
        // Draw the UI
        terminal.draw(|f| render_app(f, &app))?;

        // Poll for crossterm events (keyboard/mouse) with short timeout
        if crossterm::event::poll(Duration::from_millis(16))? {
            let evt = event::read()?;
            match evt {
                Event::Key(key) => {
                    // Ctrl+C while streaming => cancel
                    if key.code == KeyCode::Char('c')
                        && key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                    {
                        if app.is_streaming {
                            if let Some(ref ct) = cancel {
                                ct.cancel();
                            }
                            app.is_streaming = false;
                            app.status_message = Some("Cancelled.".to_string());
                            continue;
                        } else {
                            break 'main;
                        }
                    }

                    // Ctrl+D on empty input => quit
                    if key.code == KeyCode::Char('d')
                        && key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        && app.input.is_empty()
                    {
                        break 'main;
                    }

                    // Enter => submit input
                    if key.code == KeyCode::Enter && !app.is_streaming {
                        let input = app.take_input();
                        if input.is_empty() {
                            continue;
                        }

                        // Check for slash command
                        if input.starts_with('/') {
                            cmd_ctx.messages = messages.clone();
                            match execute_command(&input, &mut cmd_ctx).await {
                                Some(CommandResult::Exit) => break 'main,
                                Some(CommandResult::ClearConversation) => {
                                    messages.clear();
                                    app.messages.clear();
                                    app.status_message = Some("Conversation cleared.".to_string());
                                }
                                Some(CommandResult::SetMessages(new_msgs)) => {
                                    let removed = messages.len().saturating_sub(new_msgs.len());
                                    messages = new_msgs.clone();
                                    app.messages = new_msgs;
                                    app.status_message = Some(format!(
                                        "Rewound {} message{}.",
                                        removed,
                                        if removed == 1 { "" } else { "s" }
                                    ));
                                }
                                Some(CommandResult::Message(msg)) => {
                                    app.messages.push(cc_core::types::Message::assistant(msg));
                                }
                                Some(CommandResult::ConfigChange(new_cfg)) => {
                                    cmd_ctx.config = new_cfg.clone();
                                    app.config = new_cfg;
                                    app.status_message = Some("Configuration updated.".to_string());
                                }
                                Some(CommandResult::ConfigChangeMessage(new_cfg, msg)) => {
                                    cmd_ctx.config = new_cfg.clone();
                                    app.config = new_cfg;
                                    app.status_message = Some(msg);
                                }
                                Some(CommandResult::UserMessage(msg)) => {
                                    // Inject as user turn
                                    messages.push(cc_core::types::Message::user(msg.clone()));
                                    app.messages.push(cc_core::types::Message::user(msg));
                                    // Fall through to send to model
                                }
                                Some(CommandResult::StartOAuthFlow(with_claude_ai)) => {
                                    // Temporarily restore the terminal so the OAuth flow
                                    // can print URLs and read stdin interactively.
                                    cc_tui::restore_terminal(&mut terminal).ok();
                                    match oauth_flow::run_oauth_login_flow(with_claude_ai).await {
                                        Ok(result) => {
                                            app.status_message =
                                                Some("Login successful!".to_string());
                                            // Note: updating the live client with new credentials
                                            // requires a restart; inform the user.
                                            eprintln!(
                                                "\nLogin successful! Please restart claude to use the new credentials."
                                            );
                                            break 'main;
                                        }
                                        Err(e) => {
                                            eprintln!("\nLogin failed: {}", e);
                                        }
                                    }
                                    // Re-setup terminal
                                    terminal = cc_tui::setup_terminal()?;
                                }
                                Some(CommandResult::Error(e)) => {
                                    app.status_message = Some(format!("Error: {}", e));
                                }
                                Some(CommandResult::Silent) | None => {}
                            }
                            continue;
                        }

                        // Fire UserPromptSubmit hook (non-blocking)
                        if !config.hooks.is_empty() {
                            let hook_ctx = cc_core::hooks::HookContext {
                                event: "UserPromptSubmit".to_string(),
                                tool_name: None,
                                tool_input: None,
                                tool_output: Some(input.clone()),
                                is_error: None,
                                session_id: Some(tool_ctx.session_id.clone()),
                            };
                            cc_core::hooks::run_hooks(
                                &config.hooks,
                                cc_core::config::HookEvent::UserPromptSubmit,
                                &hook_ctx,
                                &tool_ctx.working_dir,
                            )
                            .await;
                        }

                        // Regular user message
                        messages.push(cc_core::types::Message::user(input.clone()));
                        app.messages
                            .push(cc_core::types::Message::user(input.clone()));

                        // Start async query
                        app.is_streaming = true;
                        app.streaming_text.clear();

                        let ct = CancellationToken::new();
                        cancel = Some(ct.clone());

                        // Use Arc<Mutex> so the task can write updated messages back
                        let msgs_arc = Arc::new(tokio::sync::Mutex::new(messages.clone()));
                        let msgs_arc_clone = msgs_arc.clone();

                        // Share the Arc so the spawned task can access all tools (incl. MCP).
                        let tools_arc_clone = tools_arc.clone();
                        let ctx_clone = tool_ctx.clone();
                        let qcfg = query_config.clone();
                        let tracker = cost_tracker.clone();
                        let tx = event_tx.clone();
                        let client_clone = client.clone();

                        let handle = tokio::spawn(async move {
                            let mut msgs = msgs_arc_clone.lock().await.clone();
                            let outcome = cc_query::run_query_loop(
                                client_clone.as_ref(),
                                &mut msgs,
                                tools_arc_clone.as_slice(),
                                &ctx_clone,
                                &qcfg,
                                tracker,
                                Some(tx),
                                ct,
                            )
                            .await;
                            // Write updated messages (with tool calls + assistant response) back
                            *msgs_arc_clone.lock().await = msgs;
                            outcome
                        });

                        // Store the Arc so we can read messages after task completes
                        current_query = Some((handle, msgs_arc));
                        continue;
                    }

                    app.handle_key_event(key);
                }
                Event::Resize(_, _) => {
                    // Terminal resize - will be handled on next draw
                }
                _ => {}
            }
        }

        // Drain query events
        while let Ok(evt) = event_rx.try_recv() {
            app.handle_query_event(evt);
        }

        // Check if query task is done; sync messages from the task
        let task_finished = current_query
            .as_ref()
            .map(|(h, _)| h.is_finished())
            .unwrap_or(false);

        if task_finished {
            if let Some((handle, msgs_arc)) = current_query.take() {
                // Get the outcome (ignore errors for now)
                let _ = handle.await;
                // Sync the updated conversation back to our local vector
                messages = msgs_arc.lock().await.clone();
                app.is_streaming = false;
                app.status_message = None;

                // Save session
                let session = cc_core::history::ConversationSession {
                    id: tool_ctx.session_id.clone(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    messages: messages.clone(),
                    model: config.effective_model().to_string(),
                    title: None,
                    working_dir: Some(tool_ctx.working_dir.display().to_string()),
                };
                let _ = cc_core::history::save_session(&session).await;
            }
        }

        if app.should_quit {
            break 'main;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `claude auth` subcommand handler
// ---------------------------------------------------------------------------
// Mirrors TypeScript cli.tsx `if (args[0] === 'auth') { ... }` fast-path.
// Called before Cli::parse() so it doesn't conflict with positional `prompt`.
//
// Usage:
//   claude auth login [--console]   — OAuth PKCE login (claude.ai by default)
//   claude auth logout              — Clear stored credentials
//   claude auth status [--json]     — Show authentication status

async fn handle_auth_command(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("login") => {
            // --console flag selects the Console OAuth flow (creates an API key)
            // Default (no flag) uses the Claude.ai flow (Bearer token)
            let login_with_claude_ai = !args.iter().any(|a| a == "--console");
            println!("Starting authentication...");
            match oauth_flow::run_oauth_login_flow(login_with_claude_ai).await {
                Ok(result) => {
                    println!("Successfully logged in!");
                    if let Some(email) = &result.tokens.email {
                        println!("  Account: {}", email);
                    }
                    if result.use_bearer_auth {
                        println!("  Auth method: claude.ai");
                    } else {
                        println!("  Auth method: console (API key)");
                    }
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Login failed: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Some("logout") => {
            auth_logout().await;
        }

        Some("status") => {
            let json_output = args.iter().any(|a| a == "--json");
            auth_status(json_output).await;
        }

        Some(unknown) => {
            eprintln!("Unknown auth subcommand: '{}'", unknown);
            eprintln!();
            eprintln!("Usage: claude auth <subcommand>");
            eprintln!(
                "  login [--console]   Authenticate (claude.ai by default; --console for API key)"
            );
            eprintln!("  logout              Remove stored credentials");
            eprintln!("  status [--json]     Show authentication status");
            std::process::exit(1);
        }

        None => {
            eprintln!("Usage: claude auth <login|logout|status>");
            eprintln!("  login [--console]   Authenticate with Anthropic");
            eprintln!("  logout              Remove stored credentials");
            eprintln!("  status [--json]     Show authentication status");
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Print current auth status, then exit with code 0 (logged in) or 1 (not logged in).
async fn auth_status(json_output: bool) {
    // Gather auth state
    let env_api_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    let settings = Settings::load().await.unwrap_or_default();
    let settings_api_key = settings.config.api_key.clone().filter(|k| !k.is_empty());
    let oauth_tokens = cc_core::oauth::OAuthTokens::load().await;

    // Determine auth method (mirrors TypeScript authStatus())
    let (auth_method, logged_in) = if let Some(ref tokens) = oauth_tokens {
        let uses_bearer = tokens.uses_bearer_auth();
        let method = if uses_bearer {
            "claude.ai"
        } else {
            "oauth_token"
        };
        (method.to_string(), true)
    } else if env_api_key.is_some() {
        ("api_key".to_string(), true)
    } else if settings_api_key.is_some() {
        ("api_key".to_string(), true)
    } else {
        ("none".to_string(), false)
    };

    if json_output {
        // JSON output (used by SDK + scripts)
        let mut obj = serde_json::json!({
            "loggedIn": logged_in,
            "authMethod": auth_method,
        });

        // Include API key source if known
        if env_api_key.is_some() {
            obj["apiKeySource"] = serde_json::Value::String("ANTHROPIC_API_KEY".to_string());
        } else if settings_api_key.is_some() {
            obj["apiKeySource"] = serde_json::Value::String("settings".to_string());
        }

        // Include OAuth account details for claude.ai flow
        if auth_method == "claude.ai" {
            if let Some(ref tokens) = oauth_tokens {
                obj["email"] = json_null_or_string(&tokens.email);
                obj["orgId"] = json_null_or_string(&tokens.organization_uuid);
                obj["subscriptionType"] = json_null_or_string(&tokens.subscription_type);
            }
        }

        println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
    } else {
        // Human-readable text output
        if !logged_in {
            println!("Not logged in. Run `claude auth login` to authenticate.");
        } else {
            println!("Logged in.");
            match auth_method.as_str() {
                "claude.ai" | "oauth_token" => {
                    if let Some(ref tokens) = oauth_tokens {
                        if let Some(ref email) = tokens.email {
                            println!("  Account: {}", email);
                        }
                        if let Some(ref org) = tokens.organization_uuid {
                            println!("  Organization: {}", org);
                        }
                        if let Some(ref sub) = tokens.subscription_type {
                            println!("  Subscription: {}", sub);
                        }
                        let method_label = if tokens.uses_bearer_auth() {
                            "claude.ai"
                        } else {
                            "Console (OAuth API key)"
                        };
                        println!("  Auth method: {}", method_label);
                    }
                }
                "api_key" => {
                    let source = if env_api_key.is_some() {
                        "ANTHROPIC_API_KEY (environment variable)"
                    } else {
                        "settings.json"
                    };
                    println!("  API key: {}", source);
                }
                _ => {}
            }
        }
    }

    std::process::exit(if logged_in { 0 } else { 1 });
}

/// Clear all stored credentials and exit.
async fn auth_logout() {
    let mut had_error = false;

    // Clear OAuth tokens
    if let Err(e) = cc_core::oauth::OAuthTokens::clear().await {
        eprintln!("Warning: failed to clear OAuth tokens: {}", e);
        had_error = true;
    }

    // Also clear any API key stored in settings.json
    match Settings::load().await {
        Ok(mut settings) => {
            if settings.config.api_key.is_some() {
                settings.config.api_key = None;
                if let Err(e) = settings.save().await {
                    eprintln!("Warning: failed to update settings.json: {}", e);
                    had_error = true;
                }
            }
        }
        Err(e) => {
            eprintln!("Warning: failed to load settings.json: {}", e);
        }
    }

    if had_error {
        eprintln!("Logout completed with warnings.");
        std::process::exit(1);
    } else {
        println!("Successfully logged out from your Anthropic account.");
        std::process::exit(0);
    }
}

/// Helper: convert `Option<String>` to a JSON string or null.
fn json_null_or_string(opt: &Option<String>) -> serde_json::Value {
    match opt {
        Some(s) => serde_json::Value::String(s.clone()),
        None => serde_json::Value::Null,
    }
}
