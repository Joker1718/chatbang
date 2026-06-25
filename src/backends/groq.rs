//! Groq free-tier backend (CB-012).
//!
//! Groq exposes an OpenAI-compatible API at `https://api.groq.com/openai/v1`.
//! Sign up at https://console.groq.com for a free API key.

use anyhow::Result;
use async_trait::async_trait;

use super::openai_compat::{OpenAiCompatBackend, OpenAiCompatConfig};
use super::{Backend, Message};

const GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
const DEFAULT_MODEL: &str = "llama-3.1-8b-instant";

pub struct GroqBackend {
    inner: OpenAiCompatBackend,
}

pub struct GroqConfig {
    pub api_key: String,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl GroqBackend {
    pub fn new(cfg: GroqConfig) -> Self {
        let cfg = OpenAiCompatConfig {
            base_url: GROQ_BASE_URL.into(),
            api_key: cfg.api_key,
            model: cfg.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
        };
        Self {
            inner: OpenAiCompatBackend::new(cfg, "groq"),
        }
    }
}

#[async_trait]
impl Backend for GroqBackend {
    fn name(&self) -> &str {
        "groq"
    }

    async fn chat(&self, messages: &[Message]) -> Result<String> {
        self.inner.chat(messages).await
    }
}
