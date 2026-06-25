//! Google Gemini free-tier backend (CB-012).
//!
//! Gemini has its own native request format, but it also exposes an
//! OpenAI-compatible endpoint at
//! `https://generativelanguage.googleapis.com/v1beta/openai/`. We use the
//! OpenAI-compatible shim so we can reuse the shared `openai_compat` client.
//!
//! Get a free API key at https://aistudio.google.com/apikey.

use anyhow::Result;
use async_trait::async_trait;

use super::openai_compat::{OpenAiCompatBackend, OpenAiCompatConfig};
use super::{Backend, Message};

const GEMINI_BASE_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai";
const DEFAULT_MODEL: &str = "gemini-1.5-flash";

pub struct GeminiBackend {
    inner: OpenAiCompatBackend,
}

pub struct GeminiConfig {
    pub api_key: String,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl GeminiBackend {
    pub fn new(cfg: GeminiConfig) -> Self {
        let cfg = OpenAiCompatConfig {
            base_url: GEMINI_BASE_URL.into(),
            api_key: cfg.api_key,
            model: cfg.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
        };
        Self {
            inner: OpenAiCompatBackend::new(cfg, "gemini"),
        }
    }
}

#[async_trait]
impl Backend for GeminiBackend {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn chat(&self, messages: &[Message]) -> Result<String> {
        self.inner.chat(messages).await
    }
}
