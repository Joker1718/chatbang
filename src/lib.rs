//! chatbang — access LLMs from your terminal, with a pluggable tool system.
//!
//! Backends: Ollama (local), Groq, Gemini, Together.ai, OpenAI, and the
//! legacy ChatGPT-web-scraping path (behind the `browser` feature).
//! Tool system: dynamic `ToolRegistry`, async execution, JSON Schema
//! validation, streaming output, structured `ToolError` types, and a
//! `declare_tool!` macro for low-boilerplate tool definitions.

pub mod agent;
pub mod backends;
pub mod config;
pub mod platform;
pub mod tools;

use anyhow::Result;
use clap::{Parser, Subcommand};

use agent::{run, AgentConfig};
use config::{build_backend, resolve_backend, BackendKind, ChatbangConfig};
use tools::builtins;
use tools::registry::ToolRegistry;

#[derive(Parser)]
#[command(
    name = "chatbang",
    version,
    about = "Access LLMs from your terminal — local (Ollama) or cloud (Groq/Gemini/Together) — with a pluggable tool system.",
    long_about = None
)]
pub struct Cli {
    /// Disable the agentic tool-use loop (raw chat only).
    #[arg(long)]
    pub no_agent: bool,

    /// Select the inference backend.
    /// One of: ollama, groq, gemini, together, openai, browser.
    #[arg(long, value_parser = ["ollama","groq","gemini","together","openai","browser"])]
    pub backend: Option<String>,

    /// Override the model identifier for the chosen backend.
    #[arg(long)]
    pub model: Option<String>,

    /// API key for cloud backends. If omitted, falls back to the
    /// provider-specific env var (GROQ_API_KEY, GEMINI_API_KEY, etc.).
    #[arg(long)]
    pub api_key: Option<String>,

    /// Override the backend's base URL. Mainly useful for Ollama running on
    /// a non-default port, or for self-hosted OpenAI-compatible servers.
    #[arg(long)]
    pub base_url: Option<String>,

    /// Sampling temperature (0.0–2.0, backend-specific).
    #[arg(long)]
    pub temperature: Option<f32>,

    /// Maximum number of tokens to generate.
    #[arg(long)]
    pub max_tokens: Option<u32>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Open a browser session to log in to ChatGPT and grant clipboard
    /// permission. Run this once before your first chat (browser backend only).
    Config,
    /// List the tools currently registered in the built-in tool registry.
    /// Useful for verifying that a tool is wired up correctly.
    Tools,
}

/// The CLI entry point. Exposed so integration tests can drive it
/// programmatically if needed; the binary `main.rs` is just a thin wrapper.
pub async fn cli_main() -> Result<()> {
    let cli = Cli::parse();

    let cfg = ChatbangConfig {
        backend: cli
            .backend
            .as_deref()
            .and_then(BackendKind::from_str),
        model: cli.model,
        api_key: cli.api_key,
        base_url: cli.base_url,
        temperature: cli.temperature,
        max_tokens: cli.max_tokens,
    };

    match cli.command {
        Some(Commands::Config) => {
            config::run_login_session().await?;
        }
        Some(Commands::Tools) => {
            let reg = ToolRegistry::new();
            builtins::register_all(&reg);
            let defs = reg.definitions();
            if defs.is_empty() {
                println!("(no tools registered)");
            } else {
                println!("Registered tools ({}):", defs.len());
                for d in defs {
                    println!("\n  {} — {}", d.name, d.description);
                    let schema = serde_json::to_string_pretty(&d.parameters)
                        .unwrap_or_else(|_| "{}".into());
                    println!("    schema: {}", schema);
                }
            }
        }
        None => {
            let backend_kind = resolve_backend(&cfg);
            eprintln!("[chatbang] using backend: {:?}", backend_kind);

            let backend = build_backend(&cfg)?;

            let registry = ToolRegistry::new();
            builtins::register_all(&registry);

            let agent_cfg = AgentConfig {
                agent: !cli.no_agent,
                ..Default::default()
            };

            run(backend.as_ref(), &registry, &agent_cfg).await?;
        }
    }

    Ok(())
}
