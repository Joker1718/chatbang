//! Configuration loading and CLI flag parsing.
//!
//! Handles:
//!  - Reading the `chatbang.ini` file (browser path for the legacy backend).
//!  - Resolving the active backend from CLI flags / env vars.
//!  - Constructing the concrete `Backend` instance.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::backends::{gemini, groq, ollama, together, Backend};

/// Which backend should serve chat completions?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// Legacy ChatGPT web scraping via Chromium.
    Browser,
    /// Local Ollama daemon.
    Ollama,
    /// OpenAI official API (also uses openai_compat).
    OpenAi,
    /// Groq free-tier API.
    Groq,
    /// Google Gemini free-tier API.
    Gemini,
    /// Together.ai free-tier API.
    Together,
}

impl BackendKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "browser" | "chatgpt" | "web" => Some(Self::Browser),
            "ollama" | "local" => Some(Self::Ollama),
            "openai" => Some(Self::OpenAi),
            "groq" => Some(Self::Groq),
            "gemini" | "google" => Some(Self::Gemini),
            "together" | "togetherai" => Some(Self::Together),
            _ => None,
        }
    }
}

/// All knobs the user can set via CLI or env.
#[derive(Debug, Clone, Default)]
pub struct ChatbangConfig {
    /// Selected backend. Defaults to `Ollama` if no `--backend` flag and no
    /// `CHATBANG_BACKEND` env var is set.
    pub backend: Option<BackendKind>,
    /// Model identifier (overrides the backend's default).
    pub model: Option<String>,
    /// API key (for OpenAI-compatible cloud backends).
    pub api_key: Option<String>,
    /// Base URL override (for Ollama when running on a non-default port).
    pub base_url: Option<String>,
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Max output tokens.
    pub max_tokens: Option<u32>,
}

/// Resolve CLI/env config into the effective `BackendKind`. Falls back to
/// Ollama when nothing is specified, because it requires no API key.
pub fn resolve_backend(cfg: &ChatbangConfig) -> BackendKind {
    if let Some(b) = cfg.backend {
        return b;
    }
    if let Ok(env) = std::env::var("CHATBANG_BACKEND") {
        if let Some(b) = BackendKind::from_str(&env) {
            return b;
        }
    }
    BackendKind::Ollama
}

/// Construct the concrete `Backend` from config.
pub fn build_backend(cfg: &ChatbangConfig) -> Result<Box<dyn Backend>> {
    let kind = resolve_backend(cfg);
    Ok(match kind {
        BackendKind::Ollama => Box::new(ollama::OllamaBackend::new(ollama::OllamaConfig {
            base_url: cfg.base_url.clone(),
            model: cfg.model.clone(),
            temperature: cfg.temperature,
        })),
        BackendKind::Groq => {
            let key = cfg
                .api_key
                .clone()
                .or_else(|| std::env::var("GROQ_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow!(
                        "Groq backend requires an API key. Pass --api-key or set GROQ_API_KEY."
                    )
                })?;
            Box::new(groq::GroqBackend::new(groq::GroqConfig {
                api_key: key,
                model: cfg.model.clone(),
                temperature: cfg.temperature,
                max_tokens: cfg.max_tokens,
            }))
        }
        BackendKind::Gemini => {
            let key = cfg
                .api_key
                .clone()
                .or_else(|| std::env::var("GEMINI_API_KEY").ok())
                .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow!(
                        "Gemini backend requires an API key. Pass --api-key or set GEMINI_API_KEY."
                    )
                })?;
            Box::new(gemini::GeminiBackend::new(gemini::GeminiConfig {
                api_key: key,
                model: cfg.model.clone(),
                temperature: cfg.temperature,
                max_tokens: cfg.max_tokens,
            }))
        }
        BackendKind::Together => {
            let key = cfg
                .api_key
                .clone()
                .or_else(|| std::env::var("TOGETHER_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow!(
                        "Together backend requires an API key. Pass --api-key or set TOGETHER_API_KEY."
                    )
                })?;
            Box::new(together::TogetherBackend::new(together::TogetherConfig {
                api_key: key,
                model: cfg.model.clone(),
                temperature: cfg.temperature,
                max_tokens: cfg.max_tokens,
            }))
        }
        BackendKind::OpenAi => {
            let key = cfg
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow!(
                        "OpenAI backend requires an API key. Pass --api-key or set OPENAI_API_KEY."
                    )
                })?;
            Box::new(crate::backends::openai_compat::OpenAiCompatBackend::new(
                crate::backends::openai_compat::OpenAiCompatConfig {
                    base_url: cfg
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.openai.com/v1".into()),
                    api_key: key,
                    model: cfg
                        .model
                        .clone()
                        .unwrap_or_else(|| "gpt-4o-mini".into()),
                    temperature: cfg.temperature,
                    max_tokens: cfg.max_tokens,
                },
                "openai",
            ))
        }
        BackendKind::Browser => {
            #[cfg(feature = "browser")]
            {
                let (profile_dir, browser_exe) = load_browser_config()?;
                let backend =
                    crate::backends::browser::from_paths(browser_exe, profile_dir);
                Box::new(backend)
            }
            #[cfg(not(feature = "browser"))]
            {
                return Err(anyhow!(
                    "Browser backend requires the `browser` feature, which is \
                     not enabled in this build. Use a different backend \
                     (--backend ollama|groq|gemini|together)."
                ));
            }
        }
    })
}

// ─── Legacy browser-config helpers ──────────────────────────────────────────

fn chatbang_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chatbang")
}

fn load_browser_config() -> Result<(PathBuf, String)> {
    let dir = chatbang_config_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create config dir: {}", dir.display()))?;

    let config_path = dir.join("chatbang.ini");
    let profile_dir = dir.join("profile_data");

    let saved_browser = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)
            .with_context(|| "Cannot read config file")?;
        raw.lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .find_map(|line| {
                let mut parts = line.splitn(2, '=');
                let key = parts.next()?.trim();
                let val = parts.next()?.trim();
                (key == "browser" && !val.is_empty()).then(|| val.to_string())
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    let browser = if !saved_browser.is_empty() && Path::new(&saved_browser).exists() {
        saved_browser
    } else {
        let found = crate::platform::detect_browser().ok_or_else(|| {
            anyhow!(
                "No Chromium-based browser found.\n\
                 Install Chrome, Edge, or Brave, then optionally set\n\
                 browser=<path>  in:  {}",
                config_path.display()
            )
        })?;
        let path_str = found.to_string_lossy().to_string();
        std::fs::write(&config_path, format!("browser={}\n", path_str))
            .with_context(|| "Cannot write config file")?;
        println!("Auto-detected browser: {}", path_str);
        path_str
    };

    Ok((profile_dir, browser))
}

/// Open a browser session so the user can log in to ChatGPT and grant
/// clipboard permission. Run this once before the first chat.
pub async fn run_login_session() -> Result<()> {
    #[cfg(feature = "browser")]
    {
        let (profile_dir, browser_exe) = load_browser_config()?;
        let backend = crate::backends::browser::from_paths(browser_exe, profile_dir);
        backend.login_session().await
    }
    #[cfg(not(feature = "browser"))]
    {
        Err(anyhow!(
            "Browser login session requires the `browser` feature."
        ))
    }
}
