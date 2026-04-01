# Local LLM Setup

This fork can now talk to a local Anthropic-compatible backend without Claude OAuth.

The easiest path is `llama.cpp` server, because it can expose `POST /v1/messages`,
which matches what this CLI already speaks.

## Why `llama.cpp` instead of `bitnet.cpp`

`bitnet.cpp` is a good inference runtime for 1-bit models, but this CLI expects an
Anthropic-style HTTP API. `llama.cpp` is the easier drop-in backend when you want
the fewest code changes.

If you have `BitDistill-Qwen-3-Coder-7B` as a GGUF model that runs in `llama.cpp`,
use that first.

## Start a local server

Example:

```powershell
llama-server -m C:\models\BitDistill-Qwen-3-Coder-7B.gguf --host 127.0.0.1 --port 8080
```

Use whatever flags your build of `llama.cpp` needs for GPU layers, context size,
or chat template handling.

## Run this CLI against the local server

```powershell
$env:ANTHROPIC_BASE_URL='http://127.0.0.1:8080'
$env:ANTHROPIC_API_KEY='local'
.\claude.exe --model BitDistill-Qwen-3-Coder-7B
```

You can also skip `ANTHROPIC_API_KEY` now. When `ANTHROPIC_BASE_URL` points to
`localhost` or `127.0.0.1`, the CLI uses a harmless placeholder credential instead
of trying OAuth or reusing saved Claude tokens.

## Notes

- Local backends do not support every Anthropic feature equally well.
- Tool use quality depends on the model, not just the server.
- This CLI now avoids sending saved Claude OAuth credentials to local backends.
- `anthropic-beta` headers are omitted for local backends to improve compatibility.
