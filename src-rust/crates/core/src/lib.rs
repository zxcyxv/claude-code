// cc-core: Core types, error handling, configuration, settings, and constants
// for the Claude Code CLI Rust port.
//
// All sub-modules are defined inline below.

// Re-export commonly used types at the crate root
pub use config::{Config, McpServerConfig, OutputFormat, PermissionMode, Settings, Theme};
pub use cost::CostTracker;
pub use error::{ClaudeError, Result};
pub use history::ConversationSession;
pub use permissions::{PermissionDecision, PermissionHandler, PermissionRequest};
pub use types::{
    CitationsConfig, ContentBlock, DocumentSource, ImageSource, Message, MessageContent,
    MessageCost, Role, ToolDefinition, ToolResultContent, UsageInfo,
};

// ---------------------------------------------------------------------------
// error module
// ---------------------------------------------------------------------------
pub mod error {
    use thiserror::Error;

    /// The unified error type for the Claude Code Rust port.
    #[derive(Error, Debug)]
    pub enum ClaudeError {
        #[error("API error: {0}")]
        Api(String),

        #[error("API error {status}: {message}")]
        ApiStatus { status: u16, message: String },

        #[error("Authentication error: {0}")]
        Auth(String),

        #[error("Permission denied: {0}")]
        PermissionDenied(String),

        #[error("Tool error: {0}")]
        Tool(String),

        #[error("IO error: {0}")]
        Io(#[from] std::io::Error),

        #[error("JSON error: {0}")]
        Json(#[from] serde_json::Error),

        #[error("HTTP error: {0}")]
        Http(#[from] reqwest::Error),

        #[error("Rate limit exceeded")]
        RateLimit,

        #[error("Context window exceeded")]
        ContextWindowExceeded,

        #[error("Max tokens reached")]
        MaxTokensReached,

        #[error("Cancelled")]
        Cancelled,

        #[error("Configuration error: {0}")]
        Config(String),

        #[error("MCP error: {0}")]
        Mcp(String),

        #[error("{0}")]
        Other(String),
    }

    /// Convenience alias used throughout the project.
    pub type Result<T> = std::result::Result<T, ClaudeError>;

    impl ClaudeError {
        /// Return `true` when the caller should retry the request.
        pub fn is_retryable(&self) -> bool {
            matches!(
                self,
                ClaudeError::RateLimit
                    | ClaudeError::ApiStatus { status: 429, .. }
                    | ClaudeError::ApiStatus { status: 529, .. }
            )
        }

        /// Return `true` for errors that mean the conversation cannot continue
        /// without intervention (e.g. compaction or context-window reset).
        pub fn is_context_limit(&self) -> bool {
            matches!(
                self,
                ClaudeError::ContextWindowExceeded | ClaudeError::MaxTokensReached
            )
        }
    }
}

// ---------------------------------------------------------------------------
// types module
// ---------------------------------------------------------------------------
pub mod types {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    // ---- Roles -----------------------------------------------------------

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    pub enum Role {
        User,
        Assistant,
    }

    // ---- Content blocks --------------------------------------------------

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ContentBlock {
        Text {
            text: String,
        },
        Image {
            source: ImageSource,
        },
        ToolUse {
            id: String,
            name: String,
            input: Value,
        },
        ToolResult {
            tool_use_id: String,
            content: ToolResultContent,
            #[serde(skip_serializing_if = "Option::is_none")]
            is_error: Option<bool>,
        },
        Thinking {
            thinking: String,
            signature: String,
        },
        RedactedThinking {
            data: String,
        },
        Document {
            source: DocumentSource,
            #[serde(skip_serializing_if = "Option::is_none")]
            title: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            context: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            citations: Option<CitationsConfig>,
        },
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum ToolResultContent {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ImageSource {
        #[serde(rename = "type")]
        pub source_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub media_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub url: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DocumentSource {
        #[serde(rename = "type")]
        pub source_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub media_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub url: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CitationsConfig {
        pub enabled: bool,
    }

    // ---- Messages --------------------------------------------------------

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Message {
        pub role: Role,
        pub content: MessageContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub uuid: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub cost: Option<MessageCost>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum MessageContent {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    impl Message {
        /// Create a simple user text message.
        pub fn user(content: impl Into<String>) -> Self {
            Self {
                role: Role::User,
                content: MessageContent::Text(content.into()),
                uuid: None,
                cost: None,
            }
        }

        /// Create a user message composed of multiple content blocks.
        pub fn user_blocks(blocks: Vec<ContentBlock>) -> Self {
            Self {
                role: Role::User,
                content: MessageContent::Blocks(blocks),
                uuid: None,
                cost: None,
            }
        }

        /// Create a simple assistant text message.
        pub fn assistant(content: impl Into<String>) -> Self {
            Self {
                role: Role::Assistant,
                content: MessageContent::Text(content.into()),
                uuid: None,
                cost: None,
            }
        }

        /// Create an assistant message composed of multiple content blocks.
        pub fn assistant_blocks(blocks: Vec<ContentBlock>) -> Self {
            Self {
                role: Role::Assistant,
                content: MessageContent::Blocks(blocks),
                uuid: None,
                cost: None,
            }
        }

        /// Extract the first text content from this message.
        pub fn get_text(&self) -> Option<&str> {
            match &self.content {
                MessageContent::Text(t) => Some(t.as_str()),
                MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(text.as_str())
                    } else {
                        None
                    }
                }),
            }
        }

        /// Collect all text content blocks into one concatenated string.
        pub fn get_all_text(&self) -> String {
            match &self.content {
                MessageContent::Text(t) => t.clone(),
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            }
        }

        /// Return references to all `ToolUse` blocks in this message.
        pub fn get_tool_use_blocks(&self) -> Vec<&ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
                    .collect(),
                _ => vec![],
            }
        }

        /// Return references to all `ToolResult` blocks in this message.
        pub fn get_tool_result_blocks(&self) -> Vec<&ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .collect(),
                _ => vec![],
            }
        }

        /// Return references to all `Thinking` blocks in this message.
        pub fn get_thinking_blocks(&self) -> Vec<&ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::Thinking { .. }))
                    .collect(),
                _ => vec![],
            }
        }

        /// Returns all content blocks (wrapping a single text into a vec).
        pub fn content_blocks(&self) -> Vec<ContentBlock> {
            match &self.content {
                MessageContent::Text(t) => vec![ContentBlock::Text { text: t.clone() }],
                MessageContent::Blocks(b) => b.clone(),
            }
        }

        /// Check whether this message has any tool use blocks.
        pub fn has_tool_use(&self) -> bool {
            !self.get_tool_use_blocks().is_empty()
        }
    }

    // ---- Cost / usage ----------------------------------------------------

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct MessageCost {
        pub input_tokens: u64,
        pub output_tokens: u64,
        pub cache_creation_input_tokens: u64,
        pub cache_read_input_tokens: u64,
        pub cost_usd: f64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolDefinition {
        pub name: String,
        pub description: String,
        pub input_schema: Value,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct UsageInfo {
        pub input_tokens: u64,
        pub output_tokens: u64,
        #[serde(default)]
        pub cache_creation_input_tokens: u64,
        #[serde(default)]
        pub cache_read_input_tokens: u64,
    }

    impl UsageInfo {
        pub fn total_input(&self) -> u64 {
            self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
        }

        pub fn total(&self) -> u64 {
            self.total_input() + self.output_tokens
        }
    }
}

// ---------------------------------------------------------------------------
// config module
// ---------------------------------------------------------------------------
pub mod config {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::path::PathBuf;

    // ---- Hook configuration ----------------------------------------------

    /// Events that can trigger hooks.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
    #[serde(rename_all = "PascalCase")]
    pub enum HookEvent {
        /// Fires before a tool is executed.
        PreToolUse,
        /// Fires after a tool has returned its result.
        PostToolUse,
        /// Fires when the model finishes its turn (stop).
        Stop,
        /// Fires when the user submits a prompt.
        UserPromptSubmit,
        /// General-purpose notification event.
        Notification,
    }

    /// A single hook entry: a shell command to run on a specific event.
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct HookEntry {
        /// Shell command to execute. Receives event JSON on stdin.
        pub command: String,
        /// Optional tool name filter — only run for this tool (PreToolUse/PostToolUse).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tool_filter: Option<String>,
        /// If true, a non-zero exit code blocks the operation.
        #[serde(default)]
        pub blocking: bool,
    }

    /// Top-level configuration values, merged from CLI args + settings file + env.
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Config {
        pub api_key: Option<String>,
        pub model: Option<String>,
        pub max_tokens: Option<u32>,
        pub permission_mode: PermissionMode,
        pub theme: Theme,
        #[serde(default)]
        pub output_style: Option<String>,
        pub auto_compact: bool,
        pub compact_threshold: f32,
        pub verbose: bool,
        pub output_format: OutputFormat,
        pub mcp_servers: Vec<McpServerConfig>,
        pub allowed_tools: Vec<String>,
        pub disallowed_tools: Vec<String>,
        pub env: HashMap<String, String>,
        pub enable_all_mcp_servers: bool,
        pub custom_system_prompt: Option<String>,
        pub append_system_prompt: Option<String>,
        pub disable_claude_mds: bool,
        pub project_dir: Option<PathBuf>,
        #[serde(default)]
        pub workspace_paths: Vec<PathBuf>,
        /// Event hooks: map of event → list of hook commands.
        #[serde(default)]
        pub hooks: HashMap<HookEvent, Vec<HookEntry>>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub enum PermissionMode {
        #[default]
        Default,
        AcceptEdits,
        BypassPermissions,
        Plan,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    pub enum Theme {
        #[default]
        Default,
        Dark,
        Light,
        Custom(String),
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "lowercase")]
    pub enum OutputFormat {
        #[default]
        Text,
        Json,
        StreamJson,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct McpServerConfig {
        pub name: String,
        pub command: Option<String>,
        #[serde(default)]
        pub args: Vec<String>,
        #[serde(default)]
        pub env: HashMap<String, String>,
        pub url: Option<String>,
        #[serde(rename = "type", default = "default_mcp_type")]
        pub server_type: String,
    }

    fn default_mcp_type() -> String {
        "stdio".to_string()
    }

    // ---- Settings --------------------------------------------------------

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Settings {
        #[serde(default)]
        pub config: Config,
        pub version: Option<u32>,
        #[serde(default)]
        pub projects: HashMap<String, ProjectSettings>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct ProjectSettings {
        #[serde(default)]
        pub allowed_tools: Vec<String>,
        #[serde(default)]
        pub mcp_servers: Vec<McpServerConfig>,
        pub custom_system_prompt: Option<String>,
    }

    impl Config {
        /// Return `true` when the configured API base points at a local server.
        ///
        /// This is used to support local Anthropic-compatible backends such as
        /// `llama.cpp` without triggering OAuth flows or sending real Claude
        /// credentials to a localhost process.
        pub fn api_base_is_local(api_base: &str) -> bool {
            let Ok(url) = url::Url::parse(api_base) else {
                return false;
            };

            match url.host_str() {
                Some("localhost") | Some("127.0.0.1") | Some("::1") => true,
                _ => false,
            }
        }

        /// Returns `true` when the effective API base is a local Anthropic-
        /// compatible backend such as `http://localhost:8080`.
        pub fn use_local_backend(&self) -> bool {
            Self::api_base_is_local(&self.resolve_api_base())
        }

        /// Resolve the effective model, falling back to the compile-time default.
        pub fn effective_model(&self) -> &str {
            self.model
                .as_deref()
                .unwrap_or(crate::constants::DEFAULT_MODEL)
        }

        /// Resolve the effective max-tokens.
        pub fn effective_max_tokens(&self) -> u32 {
            self.max_tokens
                .unwrap_or(crate::constants::DEFAULT_MAX_TOKENS)
        }

        /// Resolve the effective compact threshold (0.0 - 1.0).
        pub fn effective_compact_threshold(&self) -> f32 {
            if self.compact_threshold > 0.0 {
                self.compact_threshold
            } else {
                crate::constants::DEFAULT_COMPACT_THRESHOLD
            }
        }

        /// Resolve the effective output style for system-prompt assembly.
        pub fn effective_output_style(&self) -> crate::system_prompt::OutputStyle {
            self.output_style
                .as_deref()
                .map(crate::system_prompt::OutputStyle::from_str)
                .unwrap_or_default()
        }

        /// Resolve the API key from the config, then from `ANTHROPIC_API_KEY`.
        pub fn resolve_api_key(&self) -> Option<String> {
            self.api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        }

        /// Async variant: also checks `~/.claude/oauth_tokens.json`.
        /// Returns `(credential, use_bearer_auth)`.
        /// - For Console OAuth flow: credential is the stored API key, bearer=false.
        /// - For Claude.ai OAuth flow: credential is the access token, bearer=true.
        /// Silently attempts token refresh when the access token is expired.
        pub async fn resolve_auth_async(&self) -> Option<(String, bool)> {
            // Highest priority: explicit api_key or env var
            if let Some(key) = self.resolve_api_key() {
                return Some((key, false));
            }
            // Local Anthropic-compatible backends should never receive saved
            // Claude OAuth credentials. Use a harmless placeholder instead.
            if self.use_local_backend() {
                return Some(("local".to_string(), false));
            }
            // Fall back to saved OAuth tokens
            let tokens = crate::oauth::OAuthTokens::load().await?;

            // If expired and we have a refresh token, attempt silent refresh.
            // Clone the refresh token up-front so we don't borrow `tokens` during the async call.
            let refresh_token_owned = tokens.refresh_token.clone();
            let tokens = if tokens.is_expired() {
                if let Some(rt) = refresh_token_owned {
                    // Inline the refresh HTTP call (cc_core can't depend on cc_cli::oauth_flow).
                    let body = serde_json::json!({
                        "grant_type": "refresh_token",
                        "refresh_token": rt,
                        "client_id": crate::oauth::CLIENT_ID,
                        "scope": crate::oauth::ALL_SCOPES.join(" "),
                    });
                    let refreshed = 'refresh: {
                        let Ok(client) = reqwest::Client::builder()
                            .timeout(std::time::Duration::from_secs(30))
                            .build()
                        else {
                            break 'refresh None;
                        };
                        let Ok(resp) = client
                            .post(crate::oauth::TOKEN_URL)
                            .header("content-type", "application/json")
                            .json(&body)
                            .send()
                            .await
                        else {
                            break 'refresh None;
                        };
                        if !resp.status().is_success() {
                            break 'refresh None;
                        }
                        let Ok(data) = resp.json::<serde_json::Value>().await else {
                            break 'refresh None;
                        };
                        let new_at = data["access_token"].as_str().unwrap_or("").to_string();
                        if new_at.is_empty() {
                            break 'refresh None;
                        }
                        let new_rt = data["refresh_token"].as_str().map(String::from);
                        let exp_in = data["expires_in"].as_u64().unwrap_or(3600);
                        let exp_ms = chrono::Utc::now().timestamp_millis() + (exp_in as i64 * 1000);
                        let scopes: Vec<String> = data["scope"]
                            .as_str()
                            .unwrap_or("")
                            .split_whitespace()
                            .map(String::from)
                            .collect();
                        let mut r = tokens.clone();
                        r.access_token = new_at;
                        if let Some(nrt) = new_rt {
                            r.refresh_token = Some(nrt);
                        }
                        r.expires_at_ms = Some(exp_ms);
                        r.scopes = scopes;
                        let _ = r.save().await;
                        Some(r)
                    };
                    refreshed.unwrap_or(tokens)
                } else {
                    tokens // expired, no refresh token → can't fix
                }
            } else {
                tokens
            };

            if let Some(cred) = tokens.effective_credential() {
                Some((cred.to_string(), tokens.uses_bearer_auth()))
            } else {
                None
            }
        }

        /// Resolve the API base URL, checking `ANTHROPIC_BASE_URL` first.
        pub fn resolve_api_base(&self) -> String {
            std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| crate::constants::ANTHROPIC_API_BASE.to_string())
        }
    }

    impl Settings {
        /// The per-user configuration directory (`~/.claude`).
        pub fn config_dir() -> PathBuf {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
        }

        /// Full path to the global settings JSON file.
        pub fn global_settings_path() -> PathBuf {
            Self::config_dir().join("settings.json")
        }

        /// Load settings from disk, returning defaults when the file is missing.
        pub async fn load() -> anyhow::Result<Self> {
            let path = Self::global_settings_path();
            if path.exists() {
                let content = tokio::fs::read_to_string(&path).await?;
                Ok(serde_json::from_str(&content).unwrap_or_default())
            } else {
                Ok(Self::default())
            }
        }

        /// Persist settings to disk.
        pub async fn save(&self) -> anyhow::Result<()> {
            let path = Self::global_settings_path();
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let content = serde_json::to_string_pretty(self)?;
            tokio::fs::write(&path, content).await?;
            Ok(())
        }

        /// Synchronous variant used by pre-session commands.
        pub fn load_sync() -> anyhow::Result<Self> {
            let path = Self::global_settings_path();
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                Ok(serde_json::from_str(&content).unwrap_or_default())
            } else {
                Ok(Self::default())
            }
        }

        /// Synchronous variant used by pre-session commands.
        pub fn save_sync(&self) -> anyhow::Result<()> {
            let path = Self::global_settings_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = serde_json::to_string_pretty(self)?;
            std::fs::write(&path, content)?;
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// constants module
// ---------------------------------------------------------------------------
pub mod constants {
    pub const APP_NAME: &str = "claude";
    pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

    // Models
    pub const DEFAULT_MODEL: &str = "claude-opus-4-6";
    pub const SONNET_MODEL: &str = "claude-sonnet-4-6";
    pub const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
    pub const OPUS_MODEL: &str = "claude-opus-4-6";

    // Token limits
    pub const DEFAULT_MAX_TOKENS: u32 = 32_000;
    pub const MAX_TOKENS_HARD_LIMIT: u32 = 65_536;
    pub const DEFAULT_COMPACT_THRESHOLD: f32 = 0.9;
    pub const MAX_TURNS_DEFAULT: u32 = 10;
    pub const MAX_TOOL_ERRORS: u32 = 3;

    // API endpoints & headers
    pub const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
    pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";
    pub const ANTHROPIC_BETA_HEADER: &str =
        "interleaved-thinking-2025-05-14,token-efficient-tools-2025-02-19,files-api-2025-04-14";

    // File system
    pub const CLAUDE_MD_FILENAME: &str = "CLAUDE.md";
    pub const SETTINGS_FILENAME: &str = "settings.json";
    pub const HISTORY_FILENAME: &str = "conversations";
    pub const CONFIG_DIR_NAME: &str = ".claude";

    // Tool names
    pub const TOOL_NAME_BASH: &str = "Bash";
    pub const TOOL_NAME_FILE_EDIT: &str = "Edit";
    pub const TOOL_NAME_FILE_READ: &str = "Read";
    pub const TOOL_NAME_FILE_WRITE: &str = "Write";
    pub const TOOL_NAME_GLOB: &str = "Glob";
    pub const TOOL_NAME_GREP: &str = "Grep";
    pub const TOOL_NAME_AGENT: &str = "Agent";
    pub const TOOL_NAME_WEB_FETCH: &str = "WebFetch";
    pub const TOOL_NAME_WEB_SEARCH: &str = "WebSearch";
    pub const TOOL_NAME_TODO_WRITE: &str = "TodoWrite";
    pub const TOOL_NAME_TASK_CREATE: &str = "TaskCreate";
    pub const TOOL_NAME_TASK_GET: &str = "TaskGet";
    pub const TOOL_NAME_TASK_UPDATE: &str = "TaskUpdate";
    pub const TOOL_NAME_TASK_LIST: &str = "TaskList";
    pub const TOOL_NAME_TASK_STOP: &str = "TaskStop";
    pub const TOOL_NAME_TASK_OUTPUT: &str = "TaskOutput";
    pub const TOOL_NAME_ENTER_PLAN_MODE: &str = "EnterPlanMode";
    pub const TOOL_NAME_EXIT_PLAN_MODE: &str = "ExitPlanMode";
    pub const TOOL_NAME_ASK_USER: &str = "AskUserQuestion";
    pub const TOOL_NAME_MCP: &str = "mcp";
    pub const TOOL_NAME_NOTEBOOK_EDIT: &str = "NotebookEdit";

    // Session ID prefixes
    pub const SESSION_ID_PREFIX_BASH: &str = "b";
    pub const SESSION_ID_PREFIX_AGENT: &str = "a";
    pub const SESSION_ID_PREFIX_TEAMMATE: &str = "t";

    // Retry budget
    pub const MAX_OUTPUT_TOKENS_RETRIES: u32 = 3;
    pub const MAX_COMPACT_RETRIES: u32 = 3;

    // Stop sequences
    pub const STOP_SEQUENCE_END_OF_TURN: &str = "\n\nHuman:";
}

// ---------------------------------------------------------------------------
// context module
// ---------------------------------------------------------------------------
pub mod context {
    use std::path::PathBuf;
    use tokio::process::Command;

    /// Builds the system-level and user-level context that gets prepended to
    /// every conversation with the model.
    pub struct ContextBuilder {
        cwd: PathBuf,
        disable_claude_mds: bool,
    }

    impl ContextBuilder {
        pub fn new(cwd: PathBuf) -> Self {
            Self {
                cwd,
                disable_claude_mds: false,
            }
        }

        pub fn disable_claude_mds(mut self, val: bool) -> Self {
            self.disable_claude_mds = val;
            self
        }

        /// System context (git status, platform, etc.)
        pub async fn build_system_context(&self) -> String {
            let mut parts = vec![];

            // Platform information
            parts.push(format!("Platform: {}", std::env::consts::OS));
            parts.push(format!("Working directory: {}", self.cwd.display()));

            if let Some(git_context) = self.get_git_context().await {
                parts.push(git_context);
            }

            parts.join("\n\n")
        }

        /// User context (date, CLAUDE.md memories, etc.)
        pub async fn build_user_context(&self) -> String {
            let mut parts = vec![];

            let date = chrono::Local::now().format("%A, %B %d, %Y").to_string();
            parts.push(format!("Today's date is {}.", date));

            if !self.disable_claude_mds {
                if let Some(claude_md) = self.find_and_read_claude_md().await {
                    parts.push(claude_md);
                }
            }

            parts.join("\n\n")
        }

        /// Gather short git status + recent log.
        async fn get_git_context(&self) -> Option<String> {
            let output = Command::new("git")
                .args(["status", "--short", "--branch"])
                .current_dir(&self.cwd)
                .output()
                .await
                .ok()?;

            if !output.status.success() {
                return None;
            }

            let status = String::from_utf8_lossy(&output.stdout).to_string();

            let log_output = Command::new("git")
                .args(["log", "--oneline", "-5"])
                .current_dir(&self.cwd)
                .output()
                .await
                .ok()?;

            let log = String::from_utf8_lossy(&log_output.stdout).to_string();

            let mut result = format!("# Git Status\n{}", status.trim());
            if !log.trim().is_empty() {
                result.push_str(&format!("\n\n# Recent Commits\n{}", log.trim()));
            }

            Some(result)
        }

        /// Walk up from cwd looking for CLAUDE.md files and the global one.
        async fn find_and_read_claude_md(&self) -> Option<String> {
            let mut claude_mds = vec![];

            // Global ~/.claude/CLAUDE.md
            if let Some(home) = dirs::home_dir() {
                let global_claude_md = home
                    .join(".claude")
                    .join(crate::constants::CLAUDE_MD_FILENAME);
                if global_claude_md.exists() {
                    if let Ok(content) = tokio::fs::read_to_string(&global_claude_md).await {
                        claude_mds.push(format!(
                            "# Memory (from {})\n{}",
                            global_claude_md.display(),
                            content
                        ));
                    }
                }
            }

            // Walk from cwd up to filesystem root, collecting CLAUDE.md
            let mut dir = Some(self.cwd.as_path());
            let mut project_mds: Vec<String> = vec![];
            while let Some(d) = dir {
                let candidate = d.join(crate::constants::CLAUDE_MD_FILENAME);
                if candidate.exists() {
                    if let Ok(content) = tokio::fs::read_to_string(&candidate).await {
                        project_mds.push(format!(
                            "# Project Memory (from {})\n{}",
                            candidate.display(),
                            content
                        ));
                    }
                }
                dir = d.parent();
            }
            // Reverse so outermost directory comes first
            project_mds.reverse();
            claude_mds.extend(project_mds);

            if claude_mds.is_empty() {
                None
            } else {
                Some(claude_mds.join("\n\n"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// permissions module
// ---------------------------------------------------------------------------
pub mod permissions {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum PermissionDecision {
        Allow,
        AllowPermanently,
        Deny,
        DenyPermanently,
    }

    #[derive(Debug, Clone)]
    pub struct PermissionRequest {
        pub tool_name: String,
        pub description: String,
        pub details: Option<String>,
        pub is_read_only: bool,
    }

    /// Trait implemented by anything that can decide whether to allow a tool
    /// invocation.
    pub trait PermissionHandler: Send + Sync {
        fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision;
        fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision;
    }

    /// Handler that auto-decides based on the configured permission mode
    /// (used in non-interactive / headless modes).
    pub struct AutoPermissionHandler {
        pub mode: crate::config::PermissionMode,
    }

    impl PermissionHandler for AutoPermissionHandler {
        fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
            use crate::config::PermissionMode;
            match self.mode {
                PermissionMode::BypassPermissions => PermissionDecision::Allow,
                PermissionMode::AcceptEdits => {
                    // AcceptEdits auto-allows all reads and writes
                    PermissionDecision::Allow
                }
                PermissionMode::Plan => {
                    // Plan mode allows reads but denies writes
                    if request.is_read_only {
                        PermissionDecision::Allow
                    } else {
                        PermissionDecision::Deny
                    }
                }
                PermissionMode::Default => {
                    // In headless/non-interactive mode, allow reads; deny writes
                    // (Interactive mode uses InteractivePermissionHandler instead)
                    if request.is_read_only {
                        PermissionDecision::Allow
                    } else {
                        PermissionDecision::Deny
                    }
                }
            }
        }

        fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
            self.check_permission(request)
        }
    }

    /// Permission handler for interactive (TUI) mode.
    /// In Default mode, allows all operations since the user is watching the screen.
    /// In Plan mode, still restricts writes as expected.
    pub struct InteractivePermissionHandler {
        pub mode: crate::config::PermissionMode,
    }

    impl PermissionHandler for InteractivePermissionHandler {
        fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
            use crate::config::PermissionMode;
            match self.mode {
                PermissionMode::Plan => {
                    if request.is_read_only {
                        PermissionDecision::Allow
                    } else {
                        PermissionDecision::Deny
                    }
                }
                // In all other modes (Default, AcceptEdits, BypassPermissions),
                // allow all since the user is interactively watching.
                _ => PermissionDecision::Allow,
            }
        }

        fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
            self.check_permission(request)
        }
    }
}

// ---------------------------------------------------------------------------
// history module
// ---------------------------------------------------------------------------
pub mod history {
    use crate::types::Message;
    use serde::{Deserialize, Serialize};

    /// A single persisted conversation session.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ConversationSession {
        pub id: String,
        pub created_at: chrono::DateTime<chrono::Utc>,
        pub updated_at: chrono::DateTime<chrono::Utc>,
        pub messages: Vec<Message>,
        pub model: String,
        pub title: Option<String>,
        pub working_dir: Option<String>,
    }

    impl ConversationSession {
        pub fn new(model: String) -> Self {
            let now = chrono::Utc::now();
            Self {
                id: uuid::Uuid::new_v4().to_string(),
                created_at: now,
                updated_at: now,
                messages: vec![],
                model,
                title: None,
                working_dir: None,
            }
        }

        pub fn add_message(&mut self, message: Message) {
            self.messages.push(message);
            self.updated_at = chrono::Utc::now();
        }

        pub fn message_count(&self) -> usize {
            self.messages.len()
        }

        pub fn last_user_message(&self) -> Option<&Message> {
            self.messages
                .iter()
                .rev()
                .find(|m| m.role == crate::types::Role::User)
        }
    }

    /// Save a session to `~/.claude/conversations/<id>.json`.
    pub async fn save_session(session: &ConversationSession) -> anyhow::Result<()> {
        let dir = crate::config::Settings::config_dir().join("conversations");
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("{}.json", session.id));
        let content = serde_json::to_string_pretty(session)?;
        tokio::fs::write(&path, content).await?;
        Ok(())
    }

    /// Load a specific session by ID.
    pub async fn load_session(id: &str) -> anyhow::Result<ConversationSession> {
        let path = crate::config::Settings::config_dir()
            .join("conversations")
            .join(format!("{}.json", id));
        let content = tokio::fs::read_to_string(&path).await?;
        Ok(serde_json::from_str(&content)?)
    }

    /// List all sessions, sorted by most-recently-updated first.
    pub async fn list_sessions() -> anyhow::Result<Vec<ConversationSession>> {
        let dir = crate::config::Settings::config_dir().join("conversations");
        if !dir.exists() {
            return Ok(vec![]);
        }

        let mut sessions = vec![];
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(session) = serde_json::from_str::<ConversationSession>(&content) {
                        sessions.push(session);
                    }
                }
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    /// Delete a session by ID.
    pub async fn delete_session(id: &str) -> anyhow::Result<()> {
        let path = crate::config::Settings::config_dir()
            .join("conversations")
            .join(format!("{}.json", id));
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// cost module
// ---------------------------------------------------------------------------
pub mod cost {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// Per-model pricing tiers (USD per million tokens).
    #[derive(Debug, Clone, Copy)]
    pub struct ModelPricing {
        pub input_per_mtk: f64,
        pub output_per_mtk: f64,
        pub cache_creation_per_mtk: f64,
        pub cache_read_per_mtk: f64,
    }

    impl ModelPricing {
        /// Pricing for Claude Opus 4 family.
        pub const OPUS: Self = Self {
            input_per_mtk: 15.0,
            output_per_mtk: 75.0,
            cache_creation_per_mtk: 18.75,
            cache_read_per_mtk: 1.5,
        };

        /// Pricing for Claude Sonnet 4 family.
        pub const SONNET: Self = Self {
            input_per_mtk: 3.0,
            output_per_mtk: 15.0,
            cache_creation_per_mtk: 3.75,
            cache_read_per_mtk: 0.3,
        };

        /// Pricing for Claude Haiku family.
        pub const HAIKU: Self = Self {
            input_per_mtk: 0.80,
            output_per_mtk: 4.0,
            cache_creation_per_mtk: 1.0,
            cache_read_per_mtk: 0.08,
        };

        /// Default pricing is Opus (most capable, highest cost).
        pub fn default_pricing() -> Self {
            Self::OPUS
        }

        /// Pick pricing based on model name substring matching.
        pub fn for_model(model: &str) -> Self {
            if model.contains("opus") {
                Self::OPUS
            } else if model.contains("haiku") {
                Self::HAIKU
            } else {
                // Default to Sonnet pricing for unknown models
                Self::SONNET
            }
        }
    }

    impl Default for ModelPricing {
        fn default() -> Self {
            Self::OPUS
        }
    }

    /// Thread-safe, lock-free cost tracker that accumulates token usage.
    #[derive(Debug, Default)]
    pub struct CostTracker {
        input_tokens: AtomicU64,
        output_tokens: AtomicU64,
        cache_creation_tokens: AtomicU64,
        cache_read_tokens: AtomicU64,
        pricing: parking_lot::RwLock<ModelPricing>,
    }

    // We need a default for RwLock<ModelPricing> -- use Opus as default.
    impl CostTracker {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                pricing: parking_lot::RwLock::new(ModelPricing::OPUS),
                ..Default::default()
            })
        }

        pub fn with_model(model: &str) -> Arc<Self> {
            Arc::new(Self {
                pricing: parking_lot::RwLock::new(ModelPricing::for_model(model)),
                ..Default::default()
            })
        }

        pub fn set_model(&self, model: &str) {
            *self.pricing.write() = ModelPricing::for_model(model);
        }

        pub fn add_usage(&self, input: u64, output: u64, cache_creation: u64, cache_read: u64) {
            self.input_tokens.fetch_add(input, Ordering::Relaxed);
            self.output_tokens.fetch_add(output, Ordering::Relaxed);
            self.cache_creation_tokens
                .fetch_add(cache_creation, Ordering::Relaxed);
            self.cache_read_tokens
                .fetch_add(cache_read, Ordering::Relaxed);
        }

        pub fn total_cost_usd(&self) -> f64 {
            let pricing = *self.pricing.read();
            let input = self.input_tokens.load(Ordering::Relaxed) as f64;
            let output = self.output_tokens.load(Ordering::Relaxed) as f64;
            let cache_creation = self.cache_creation_tokens.load(Ordering::Relaxed) as f64;
            let cache_read = self.cache_read_tokens.load(Ordering::Relaxed) as f64;

            (input * pricing.input_per_mtk
                + output * pricing.output_per_mtk
                + cache_creation * pricing.cache_creation_per_mtk
                + cache_read * pricing.cache_read_per_mtk)
                / 1_000_000.0
        }

        pub fn total_tokens(&self) -> u64 {
            self.input_tokens.load(Ordering::Relaxed)
                + self.output_tokens.load(Ordering::Relaxed)
                + self.cache_creation_tokens.load(Ordering::Relaxed)
                + self.cache_read_tokens.load(Ordering::Relaxed)
        }

        pub fn input_tokens(&self) -> u64 {
            self.input_tokens.load(Ordering::Relaxed)
        }

        pub fn output_tokens(&self) -> u64 {
            self.output_tokens.load(Ordering::Relaxed)
        }

        pub fn cache_creation_tokens(&self) -> u64 {
            self.cache_creation_tokens.load(Ordering::Relaxed)
        }

        pub fn cache_read_tokens(&self) -> u64 {
            self.cache_read_tokens.load(Ordering::Relaxed)
        }

        /// Produce a human-readable summary string, e.g. for display in the TUI.
        pub fn summary(&self) -> String {
            let cost = self.total_cost_usd();
            let total = self.total_tokens();
            if cost < 0.01 {
                format!("{} tokens (<$0.01)", total)
            } else {
                format!("{} tokens (${:.2})", total, cost)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// hooks module
// ---------------------------------------------------------------------------
pub mod hooks {
    use crate::config::{HookEntry, HookEvent};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::Path;
    use tracing::{debug, warn};

    /// Context passed to hook commands via stdin as JSON.
    #[derive(Debug, serde::Serialize)]
    pub struct HookContext {
        pub event: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tool_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tool_input: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tool_output: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub session_id: Option<String>,
    }

    /// Result of running a hook.
    #[derive(Debug)]
    pub enum HookOutcome {
        /// Hook ran and allowed execution to continue.
        Allowed,
        /// Hook ran and blocked execution (blocking hook with non-zero exit).
        Blocked(String),
        /// Hook produced modified output (stdout of the hook command).
        Modified(String),
    }

    /// Run all hooks registered for the given event. Returns the first blocking
    /// result if any hook blocks, otherwise `Allowed`.
    pub async fn run_hooks(
        hooks: &HashMap<HookEvent, Vec<HookEntry>>,
        event: HookEvent,
        ctx: &HookContext,
        working_dir: &Path,
    ) -> HookOutcome {
        let Some(entries) = hooks.get(&event) else {
            return HookOutcome::Allowed;
        };

        let ctx_json = match serde_json::to_string(ctx) {
            Ok(j) => j,
            Err(e) => {
                warn!("Failed to serialize hook context: {}", e);
                return HookOutcome::Allowed;
            }
        };

        for entry in entries {
            // Apply tool filter if set
            if let Some(ref filter) = entry.tool_filter {
                if let Some(ref tool) = ctx.tool_name {
                    if !filter.is_empty() && filter != tool && filter != "*" {
                        continue;
                    }
                }
            }

            debug!(command = %entry.command, event = ?event, "Running hook");

            let result = tokio::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" })
                .args(if cfg!(windows) {
                    ["/C", &entry.command]
                } else {
                    ["-c", &entry.command]
                })
                .current_dir(working_dir)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();

            let mut child = match result {
                Ok(c) => c,
                Err(e) => {
                    warn!(command = %entry.command, error = %e, "Failed to spawn hook");
                    continue;
                }
            };

            // Write context JSON to stdin
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(ctx_json.as_bytes()).await;
            }

            let output = match child.wait_with_output().await {
                Ok(o) => o,
                Err(e) => {
                    warn!(command = %entry.command, error = %e, "Hook wait failed");
                    continue;
                }
            };

            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let exit_ok = output.status.success();

            if !exit_ok && entry.blocking {
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let reason = if !stderr.is_empty() { stderr } else { stdout };
                return HookOutcome::Blocked(format!(
                    "Hook '{}' blocked execution: {}",
                    entry.command,
                    reason.trim()
                ));
            }

            if !stdout.trim().is_empty() {
                return HookOutcome::Modified(stdout.trim().to_string());
            }
        }

        HookOutcome::Allowed
    }
}

// ---------------------------------------------------------------------------
// oauth module
// ---------------------------------------------------------------------------

/// OAuth 2.0 PKCE authentication support.
///
/// Supports two login paths mirroring the TypeScript implementation:
/// - **Console** (`org:create_api_key` scope): exchanges access token for an API key.
/// - **Claude.ai** (`user:inference` scope): uses the access token as a Bearer credential.
pub mod oauth {
    use serde::{Deserialize, Serialize};

    // ---- Production OAuth endpoints & constants ----

    pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
    pub const CONSOLE_AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";
    pub const CLAUDE_AI_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
    pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
    pub const API_KEY_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/create_api_key";
    pub const MANUAL_REDIRECT_URL: &str = "https://platform.claude.com/oauth/code/callback";
    pub const CLAUDEAI_SUCCESS_URL: &str =
        "https://platform.claude.com/oauth/code/success?app=claude-code";
    pub const CONSOLE_SUCCESS_URL: &str = "https://platform.claude.com/buy_credits\
        ?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code";

    /// All scopes requested during login (union of Console + Claude.ai scopes).
    pub const ALL_SCOPES: &[&str] = &[
        "org:create_api_key",
        "user:profile",
        "user:inference",
        "user:sessions:claude_code",
        "user:mcp_servers",
        "user:file_upload",
    ];

    /// Scope that identifies a Claude.ai subscription token (uses Bearer auth).
    pub const CLAUDE_AI_INFERENCE_SCOPE: &str = "user:inference";

    // ---- Stored token struct ----

    /// Persisted OAuth tokens (saved to `~/.claude/oauth_tokens.json`).
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct OAuthTokens {
        pub access_token: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub refresh_token: Option<String>,
        /// Unix timestamp in milliseconds when the access token expires.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub expires_at_ms: Option<i64>,
        pub scopes: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub account_uuid: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub email: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub organization_uuid: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub subscription_type: Option<String>,
        /// API key created for Console-flow users (exchanged from access token).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub api_key: Option<String>,
    }

    impl OAuthTokens {
        /// Returns true if the token requires Bearer-style authorization
        /// (i.e. Claude.ai subscription with `user:inference` scope).
        pub fn uses_bearer_auth(&self) -> bool {
            self.scopes.iter().any(|s| s == CLAUDE_AI_INFERENCE_SCOPE)
        }

        /// The credential to present to the Anthropic API:
        /// - Console flow: the stored `api_key` (sk-ant-…)
        /// - Claude.ai flow: the `access_token` itself (Bearer)
        pub fn effective_credential(&self) -> Option<&str> {
            if self.uses_bearer_auth() {
                if self.access_token.is_empty() {
                    None
                } else {
                    Some(&self.access_token)
                }
            } else {
                self.api_key.as_deref()
            }
        }

        /// True if the access token has passed (or is within 5 minutes of) its expiry.
        pub fn is_expired(&self) -> bool {
            if let Some(exp) = self.expires_at_ms {
                let buffer_ms: i64 = 5 * 60 * 1000;
                let now_ms = chrono::Utc::now().timestamp_millis();
                (now_ms + buffer_ms) >= exp
            } else {
                false
            }
        }

        pub fn token_file_path() -> std::path::PathBuf {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".claude")
                .join("oauth_tokens.json")
        }

        pub async fn save(&self) -> anyhow::Result<()> {
            let path = Self::token_file_path();
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&path, serde_json::to_string_pretty(self)?).await?;
            Ok(())
        }

        pub async fn load() -> Option<Self> {
            let path = Self::token_file_path();
            let content = tokio::fs::read_to_string(&path).await.ok()?;
            serde_json::from_str(&content).ok()
        }

        pub async fn clear() -> anyhow::Result<()> {
            let path = Self::token_file_path();
            if path.exists() {
                tokio::fs::remove_file(&path).await?;
            }
            Ok(())
        }
    }

    // ---- PKCE helpers ----

    /// Generate a 32-byte random code verifier, base64url-encoded (no padding).
    pub fn generate_code_verifier() -> String {
        use base64::Engine;
        let mut bytes = [0u8; 32];
        let u1 = uuid::Uuid::new_v4();
        let u2 = uuid::Uuid::new_v4();
        bytes[..16].copy_from_slice(u1.as_bytes());
        bytes[16..].copy_from_slice(u2.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Derive the PKCE code challenge from a verifier: BASE64URL(SHA256(verifier)).
    pub fn generate_code_challenge(verifier: &str) -> String {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(verifier.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
    }

    /// Generate a random OAuth state parameter for CSRF protection.
    pub fn generate_state() -> String {
        use base64::Engine;
        let mut bytes = [0u8; 32];
        let u1 = uuid::Uuid::new_v4();
        let u2 = uuid::Uuid::new_v4();
        bytes[..16].copy_from_slice(u1.as_bytes());
        bytes[16..].copy_from_slice(u2.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    // ---- URL builder ----

    /// Build an OAuth authorization URL with all required PKCE parameters.
    pub fn build_auth_url(
        authorize_base: &str,
        code_challenge: &str,
        state: &str,
        callback_port: u16,
        is_manual: bool,
    ) -> String {
        let mut u = url::Url::parse(authorize_base).expect("valid OAuth authorize base URL");
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("code", "true");
            q.append_pair("client_id", CLIENT_ID);
            q.append_pair("response_type", "code");
            let redirect = if is_manual {
                MANUAL_REDIRECT_URL.to_string()
            } else {
                format!("http://localhost:{}/callback", callback_port)
            };
            q.append_pair("redirect_uri", &redirect);
            q.append_pair("scope", &ALL_SCOPES.join(" "));
            q.append_pair("code_challenge", code_challenge);
            q.append_pair("code_challenge_method", "S256");
            q.append_pair("state", state);
        }
        u.to_string()
    }
}

// Re-export OAuthTokens at crate root for convenience
pub use oauth::OAuthTokens;

// ---------------------------------------------------------------------------
// New modules: keybindings, voice, analytics, lsp, team_memory_sync,
//              system_prompt, memdir, oauth_config
// ---------------------------------------------------------------------------
pub mod analytics;
pub mod keybindings;
pub mod lsp;
pub mod memdir;
pub mod migrations;
pub mod oauth_config;
pub mod output_styles;
pub mod system_prompt;
pub mod team_memory_sync;
pub mod voice;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_user() {
        let msg = Message::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.get_text(), Some("hello"));
    }

    #[test]
    fn test_message_assistant_blocks() {
        let msg = Message::assistant_blocks(vec![
            ContentBlock::Thinking {
                thinking: "let me think".into(),
                signature: "sig".into(),
            },
            ContentBlock::Text {
                text: "response".into(),
            },
        ]);
        assert_eq!(msg.get_text(), Some("response"));
        assert_eq!(msg.get_thinking_blocks().len(), 1);
    }

    #[test]
    fn test_hooks_config_default() {
        let cfg = crate::config::Config::default();
        assert!(cfg.hooks.is_empty());
    }

    #[test]
    fn test_cost_tracker() {
        let tracker = CostTracker::new();
        tracker.add_usage(1000, 500, 200, 100);
        assert_eq!(tracker.input_tokens(), 1000);
        assert_eq!(tracker.output_tokens(), 500);
        assert!(tracker.total_cost_usd() > 0.0);
    }

    #[test]
    fn test_error_retryable() {
        assert!(ClaudeError::RateLimit.is_retryable());
        assert!(ClaudeError::ApiStatus {
            status: 429,
            message: "rate limited".into()
        }
        .is_retryable());
        assert!(!ClaudeError::Auth("bad key".into()).is_retryable());
    }

    // ---- Config tests -------------------------------------------------------

    #[test]
    fn test_config_effective_model_default() {
        let cfg = crate::config::Config::default();
        assert_eq!(cfg.effective_model(), crate::constants::DEFAULT_MODEL);
    }

    #[test]
    fn test_config_effective_model_override() {
        let mut cfg = crate::config::Config::default();
        cfg.model = Some("claude-haiku-4-5-20251001".to_string());
        assert_eq!(cfg.effective_model(), "claude-haiku-4-5-20251001");
    }

    #[test]
    fn test_config_effective_max_tokens_default() {
        let cfg = crate::config::Config::default();
        assert_eq!(
            cfg.effective_max_tokens(),
            crate::constants::DEFAULT_MAX_TOKENS
        );
    }

    #[test]
    fn test_config_effective_max_tokens_override() {
        let mut cfg = crate::config::Config::default();
        cfg.max_tokens = Some(8192);
        assert_eq!(cfg.effective_max_tokens(), 8192);
    }

    #[test]
    fn test_config_resolve_api_key_from_config() {
        // When config.api_key is set, it should be returned regardless of env var
        // (Config key takes priority — resolve_api_key returns it first)
        let orig = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");

        let mut cfg = crate::config::Config::default();
        cfg.api_key = Some("sk-ant-config-key".to_string());
        assert_eq!(cfg.resolve_api_key(), Some("sk-ant-config-key".to_string()));

        if let Some(k) = orig {
            std::env::set_var("ANTHROPIC_API_KEY", k);
        }
    }

    #[test]
    fn test_config_resolve_api_key_none() {
        // Temporarily ensure no env var override
        let orig = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");

        let cfg = crate::config::Config::default();
        assert!(cfg.resolve_api_key().is_none());

        // Restore
        if let Some(k) = orig {
            std::env::set_var("ANTHROPIC_API_KEY", k);
        }
    }

    #[test]
    fn test_config_resolve_api_key_from_env() {
        let orig = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-env-key");

        let cfg = crate::config::Config::default();
        assert_eq!(cfg.resolve_api_key(), Some("sk-ant-env-key".to_string()));

        // Restore
        std::env::remove_var("ANTHROPIC_API_KEY");
        if let Some(k) = orig {
            std::env::set_var("ANTHROPIC_API_KEY", k);
        }
    }

    // ---- OAuth token tests --------------------------------------------------

    #[test]
    fn test_oauth_tokens_not_expired_no_expiry() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: "at".to_string(),
            expires_at_ms: None,
            ..Default::default()
        };
        assert!(
            !tokens.is_expired(),
            "Token with no expiry should not be considered expired"
        );
    }

    #[test]
    fn test_oauth_tokens_expired_past() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: "at".to_string(),
            // Expired 1 hour ago
            expires_at_ms: Some(chrono::Utc::now().timestamp_millis() - 3_600_000),
            ..Default::default()
        };
        assert!(tokens.is_expired());
    }

    #[test]
    fn test_oauth_tokens_not_expired_future() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: "at".to_string(),
            // Expires in 1 hour
            expires_at_ms: Some(chrono::Utc::now().timestamp_millis() + 3_600_000),
            ..Default::default()
        };
        assert!(!tokens.is_expired());
    }

    #[test]
    fn test_oauth_tokens_expired_within_buffer() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: "at".to_string(),
            // Expires in 3 minutes — within the 5-minute buffer, so treated as expired
            expires_at_ms: Some(chrono::Utc::now().timestamp_millis() + 3 * 60 * 1000),
            ..Default::default()
        };
        assert!(
            tokens.is_expired(),
            "Token within 5-min buffer should be considered expired"
        );
    }

    #[test]
    fn test_oauth_uses_bearer_auth_with_inference_scope() {
        let tokens = crate::oauth::OAuthTokens {
            scopes: vec![crate::oauth::CLAUDE_AI_INFERENCE_SCOPE.to_string()],
            ..Default::default()
        };
        assert!(tokens.uses_bearer_auth());
    }

    #[test]
    fn test_oauth_uses_bearer_auth_without_inference_scope() {
        let tokens = crate::oauth::OAuthTokens {
            scopes: vec!["org:create_api_key".to_string()],
            ..Default::default()
        };
        assert!(!tokens.uses_bearer_auth());
    }

    #[test]
    fn test_oauth_effective_credential_bearer() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: "bearer_token_xyz".to_string(),
            scopes: vec![crate::oauth::CLAUDE_AI_INFERENCE_SCOPE.to_string()],
            api_key: Some("sk-ant-ignored".to_string()),
            ..Default::default()
        };
        assert_eq!(tokens.effective_credential(), Some("bearer_token_xyz"));
    }

    #[test]
    fn test_oauth_effective_credential_api_key() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: "at".to_string(),
            scopes: vec!["org:create_api_key".to_string()],
            api_key: Some("sk-ant-real-key".to_string()),
            ..Default::default()
        };
        assert_eq!(tokens.effective_credential(), Some("sk-ant-real-key"));
    }

    #[test]
    fn test_oauth_effective_credential_bearer_empty_access_token() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: String::new(),
            scopes: vec![crate::oauth::CLAUDE_AI_INFERENCE_SCOPE.to_string()],
            ..Default::default()
        };
        assert_eq!(tokens.effective_credential(), None);
    }

    #[test]
    fn test_oauth_effective_credential_no_api_key() {
        let tokens = crate::oauth::OAuthTokens {
            access_token: "at".to_string(),
            scopes: vec!["org:create_api_key".to_string()],
            api_key: None,
            ..Default::default()
        };
        assert_eq!(tokens.effective_credential(), None);
    }

    // ---- PKCE tests ---------------------------------------------------------

    #[test]
    fn test_pkce_code_verifier_length() {
        let verifier = crate::oauth::generate_code_verifier();
        // 32 bytes base64url-encoded (no padding) = ceil(32 * 4/3) = 43 chars
        assert_eq!(
            verifier.len(),
            43,
            "Code verifier should be 43 base64url chars (32 bytes)"
        );
        // Must only contain URL-safe base64 chars
        assert!(verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_pkce_code_challenge_format() {
        let verifier = crate::oauth::generate_code_verifier();
        let challenge = crate::oauth::generate_code_challenge(&verifier);
        // SHA256 = 32 bytes → 43 base64url chars
        assert_eq!(
            challenge.len(),
            43,
            "Code challenge should be 43 base64url chars (SHA256 = 32 bytes)"
        );
        assert!(challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_pkce_challenge_deterministic() {
        // Same verifier must produce same challenge
        let verifier = "test_verifier_fixed_input";
        let c1 = crate::oauth::generate_code_challenge(verifier);
        let c2 = crate::oauth::generate_code_challenge(verifier);
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_pkce_verifier_unique() {
        let v1 = crate::oauth::generate_code_verifier();
        let v2 = crate::oauth::generate_code_verifier();
        assert_ne!(v1, v2, "Code verifiers should be unique");
    }

    #[test]
    fn test_pkce_state_length_and_format() {
        let state = crate::oauth::generate_state();
        assert_eq!(state.len(), 43);
        assert!(state
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    // ---- Auth URL building tests --------------------------------------------

    #[test]
    fn test_build_auth_url_automatic_has_localhost_redirect() {
        let challenge = "test_challenge";
        let state = "test_state";
        let port: u16 = 12345;
        let url = crate::oauth::build_auth_url(
            crate::oauth::CONSOLE_AUTHORIZE_URL,
            challenge,
            state,
            port,
            false, // automatic
        );
        assert!(url.contains("redirect_uri="), "URL must have redirect_uri");
        assert!(
            url.contains("localhost%3A12345") || url.contains("localhost:12345"),
            "Automatic URL should use localhost callback"
        );
        assert!(url.contains("code_challenge=test_challenge"));
        assert!(url.contains("state=test_state"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("client_id={}", crate::oauth::CLIENT_ID)));
    }

    #[test]
    fn test_build_auth_url_manual_has_manual_redirect() {
        let url = crate::oauth::build_auth_url(
            crate::oauth::CLAUDE_AI_AUTHORIZE_URL,
            "challenge",
            "state",
            9999,
            true, // manual
        );
        assert!(url.contains("redirect_uri="), "URL must have redirect_uri");
        // Manual redirect should NOT be localhost
        assert!(
            !url.contains("localhost"),
            "Manual URL should not use localhost callback"
        );
    }

    // ---- Permission handler tests -------------------------------------------

    fn make_req(tool_name: &str, is_read_only: bool) -> crate::permissions::PermissionRequest {
        crate::permissions::PermissionRequest {
            tool_name: tool_name.to_string(),
            description: format!("{} operation", tool_name),
            details: None,
            is_read_only,
        }
    }

    #[test]
    fn test_auto_handler_bypass_allows_all() {
        let handler = crate::permissions::AutoPermissionHandler {
            mode: crate::config::PermissionMode::BypassPermissions,
        };
        assert_eq!(
            handler.check_permission(&make_req("FileWrite", false)),
            crate::permissions::PermissionDecision::Allow
        );
    }

    #[test]
    fn test_auto_handler_default_allows_reads() {
        let handler = crate::permissions::AutoPermissionHandler {
            mode: crate::config::PermissionMode::Default,
        };
        assert_eq!(
            handler.check_permission(&make_req("FileRead", true)),
            crate::permissions::PermissionDecision::Allow
        );
    }

    #[test]
    fn test_auto_handler_default_denies_writes() {
        let handler = crate::permissions::AutoPermissionHandler {
            mode: crate::config::PermissionMode::Default,
        };
        assert_eq!(
            handler.check_permission(&make_req("FileWrite", false)),
            crate::permissions::PermissionDecision::Deny
        );
    }

    #[test]
    fn test_auto_handler_accept_edits_allows_writes() {
        let handler = crate::permissions::AutoPermissionHandler {
            mode: crate::config::PermissionMode::AcceptEdits,
        };
        assert_eq!(
            handler.check_permission(&make_req("FileWrite", false)),
            crate::permissions::PermissionDecision::Allow
        );
    }

    #[test]
    fn test_auto_handler_plan_denies_writes() {
        let handler = crate::permissions::AutoPermissionHandler {
            mode: crate::config::PermissionMode::Plan,
        };
        assert_eq!(
            handler.check_permission(&make_req("Bash", false)),
            crate::permissions::PermissionDecision::Deny
        );
        assert_eq!(
            handler.check_permission(&make_req("FileRead", true)),
            crate::permissions::PermissionDecision::Allow
        );
    }

    #[test]
    fn test_interactive_handler_default_allows_writes() {
        // InteractivePermissionHandler allows writes in Default mode
        // (user is watching the TUI)
        let handler = crate::permissions::InteractivePermissionHandler {
            mode: crate::config::PermissionMode::Default,
        };
        assert_eq!(
            handler.check_permission(&make_req("FileWrite", false)),
            crate::permissions::PermissionDecision::Allow
        );
    }

    #[test]
    fn test_interactive_handler_plan_allows_reads_denies_writes() {
        let handler = crate::permissions::InteractivePermissionHandler {
            mode: crate::config::PermissionMode::Plan,
        };
        assert_eq!(
            handler.check_permission(&make_req("FileRead", true)),
            crate::permissions::PermissionDecision::Allow
        );
        assert_eq!(
            handler.check_permission(&make_req("FileWrite", false)),
            crate::permissions::PermissionDecision::Deny
        );
    }

    // ---- Message content tests ----------------------------------------------

    #[test]
    fn test_message_get_all_text_multiple_blocks() {
        let msg = Message::assistant_blocks(vec![
            ContentBlock::Text {
                text: "First ".into(),
            },
            ContentBlock::Text {
                text: "Second".into(),
            },
        ]);
        assert_eq!(msg.get_all_text(), "First Second");
    }

    #[test]
    fn test_message_get_text_returns_first_text_block() {
        let msg = Message::assistant_blocks(vec![
            ContentBlock::Thinking {
                thinking: "reasoning".into(),
                signature: "sig".into(),
            },
            ContentBlock::Text {
                text: "answer".into(),
            },
        ]);
        assert_eq!(msg.get_text(), Some("answer"));
    }

    #[test]
    fn test_message_has_tool_use_false() {
        let msg = Message::user("just text");
        assert!(!msg.has_tool_use());
    }

    #[test]
    fn test_cost_tracker_cumulative() {
        let tracker = CostTracker::new();
        tracker.add_usage(1000, 500, 100, 50);
        tracker.add_usage(200, 100, 0, 0);
        assert_eq!(tracker.input_tokens(), 1200);
        assert_eq!(tracker.output_tokens(), 600);
    }

    #[test]
    fn test_cost_tracker_initial_zero() {
        let tracker = CostTracker::new();
        assert_eq!(tracker.input_tokens(), 0);
        assert_eq!(tracker.output_tokens(), 0);
        assert_eq!(tracker.total_cost_usd(), 0.0);
    }
}
