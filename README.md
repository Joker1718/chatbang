# chatbang

Access LLMs from your terminal — local (Ollama) or cloud (Groq / Gemini /
Together.ai / OpenAI) — with a pluggable, dynamically-registered tool system.

This is the `rust-dev` branch: a ground-up rewrite of the original
Windows-only ChatGPT-web-scraping tool. The architecture is now:

- **Modular**: every concern lives in its own module (`tools/`, `backends/`,
  `platform/`, `agent.rs`, `config.rs`).
- **Backend-agnostic**: a single `Backend` trait is implemented by Ollama,
  Groq, Gemini, Together, OpenAI, and the legacy browser-scraping path.
- **Tool-registry-driven**: tools self-register at startup; the system prompt
  is auto-generated from the live registry, so the prompt and the dispatch
  table can never drift out of sync.
- **Cross-platform**: builds and runs on Linux, macOS, and Windows. Shell
  selection (`cmd` vs `sh`) and browser discovery are platform-aware.
- **Fully async**: all tool execution uses `tokio::process` / `tokio::fs`,
  so long-running commands don't block the runtime.

## Quick start (Ollama, local, no API key)

1. **Install Ollama**: <https://ollama.com> (one download, no signup).
2. Pull a model and start the daemon:
   ```bash
   ollama pull llama3.1
   ollama serve          # leaves a server on http://localhost:11434
   ```
3. Build and run chatbang:
   ```bash
   cargo build --release --no-default-features
   ./target/release/chatbang
   ```
   The first time you type a message, chatbang will reach out to the local
   Ollama daemon and chat back. Tool calls (`run_command`, `read_file`,
   `write_file`, `list_dir`) are enabled by default.

If Ollama isn't running, chatbang will print a clear error telling you how
to start it — it does **not** silently time out.

## Quick start (Groq free tier)

1. Get a free API key at <https://console.groq.com>.
2. Export it and run:
   ```bash
   export GROQ_API_KEY=...
   cargo build --release --no-default-features
   ./target/release/chatbang --backend groq
   ```

The same pattern works for the other cloud backends — just swap the env
var (`GEMINI_API_KEY`, `TOGETHER_API_KEY`, `OPENAI_API_KEY`) and the
`--backend` flag.

## CLI reference

```
chatbang [OPTIONS] [COMMAND]

Commands:
  config   Open a browser session to log in to ChatGPT (browser backend only)
  tools    List the tools currently registered in the registry
  help     Print this message

Options:
      --no-agent                   Disable the agentic tool-use loop
      --backend <BACKEND>          ollama | groq | gemini | together | openai | browser
      --model <MODEL>              Override the backend's default model
      --api-key <API_KEY>          API key for cloud backends
      --base-url <BASE_URL>        Override the backend's base URL
      --temperature <TEMPERATURE>  Sampling temperature
      --max-tokens <MAX_TOKENS>    Maximum tokens to generate
```

Backend can also be selected via the `CHATBANG_BACKEND` env var.

## Architecture

```
src/
├── main.rs                CLI parsing, subcommand dispatch
├── agent.rs               The interactive REPL + tool-use loop
├── config.rs              Resolve backend from CLI/env, build it
├── platform/mod.rs        Cross-platform shell + browser detection (CB-005)
├── backends/
│   ├── mod.rs             Backend trait, Message, Role
│   ├── openai_compat.rs   Shared OpenAI-compatible HTTP client
│   ├── ollama.rs          Local Ollama backend (CB-011)
│   ├── groq.rs            Groq free-tier (CB-012)
│   ├── gemini.rs          Google Gemini free-tier (CB-012)
│   ├── together.rs        Together.ai free-tier (CB-012)
│   └── browser.rs         Legacy ChatGPT-web-scraping (behind `browser` feature)
└── tools/
    ├── mod.rs             Re-exports
    ├── error.rs           ToolError enum (CB-008)
    ├── traits.rs          Tool trait + ToolDefinition (CB-002)
    ├── registry.rs        ToolRegistry, JSON-Schema preflight (CB-001, CB-003)
    ├── protocol.rs        JSON tool-call parsing (CB-007)
    ├── prompt.rs          Auto-generate system prompt (CB-010)
    ├── macros.rs          declare_tool! / declare_async_tool! (CB-009)
    └── builtins/
        ├── mod.rs         register_all()
        ├── run_command.rs Async + streaming (CB-004, CB-006)
        ├── read_file.rs   Async tokio::fs (CB-004)
        ├── write_file.rs  Async tokio::fs (CB-004)
        └── list_dir.rs    Async tokio::fs (CB-004)
```

## Adding a new tool

Three options, from least to most boilerplate:

### Option A — `declare_tool!` macro

```rust
use chatbang::declare_tool;

declare_tool! {
    tool: WeatherTool,
    args: WeatherArgs,
    name: "get_weather",
    description: "Get the current weather for a city",
    fields: { city: String, units: Option<String> }
    execute: |args| {
        let a: WeatherArgs = serde_json::from_value(args.clone())?;
        Ok(format!("Weather in {}: sunny, 22°C", a.city))
    }
}
```

### Option B — Hand-written struct + `Tool` impl

```rust
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use chatbang::tools::{Tool, ToolResult, ToolRegistry};

#[derive(Deserialize, JsonSchema)]
struct WeatherArgs { city: String }

struct WeatherTool;

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &str { "get_weather" }
    fn description(&self) -> &str { "Get the current weather for a city" }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(WeatherArgs)
    }
    async fn execute(&self, args: &serde_json::Value) -> ToolResult<String> {
        let a: WeatherArgs = serde_json::from_value(args.clone())?;
        Ok(format!("Weather in {}: sunny, 22°C", a.city))
    }
}

// At startup:
reg.register(WeatherTool);
```

### Option C — Streaming tool

Override `execute_streaming` to push output line-by-line through an
`mpsc::Sender<String>`. See `src/tools/builtins/run_command.rs` for a
full example.

## Feature flags

- `browser` (default): enable the legacy ChatGPT-web-scraping backend via
  `chromiumoxide`. Disable with `--no-default-features` for headless
  servers where Chromium isn't installed.

## Tests

```bash
cargo test --no-default-features
```

44 unit tests cover the tool trait, registry, protocol parser, prompt
generator, every built-in tool, the macro, and the Ollama reachability
fallback.

## License

MIT.
