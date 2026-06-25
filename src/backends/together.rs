//! Together.ai free-tier backend (CB-012).
//!
//! Together.ai exposes an OpenAI-compatible API at `https://api.together.xyz/v1`.
//! Sign up at https://api.together.ai for free initial credits.

use anyhow::Result;
use async_trait::async_trait;

use super::openai_compat::{OpenAiCompatBackend, OpenAiCompatConfig};
use super::{Backend, Message};

const TOGETHER_BASE_URL: &str = "https://api.together.xyz/v1";
const DEFAULT_MODEL: &str = "meta-llama/Llama-3.3-70B-Instruct-Turbo-Free";

pub struct TogetherBackend {
    inner: OpenAiCompatBackend,
}

pub struct TogetherConfig {
    pub api_key: String,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl TogetherBackend {
    pub fn new(cfg: TogetherConfig) -> Self {
        let cfg = OpenAiCompatConfig {
            base_url: TOGETHER_BASE_URL.into(),
            api_key: cfg.api_key,
            model: cfg.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
        };
        Self {
            inner: OpenAiCompatBackend::new(cfg, "together"),
        }
    }
}

#[async_trait]
impl Backend for TogetherBackend {
    fn name(&self) -> &str {
        "together"
    }

    async fn chat(&self, messages: &[Message]) -> Result<String> {
        self.inner.chat(messages).await
    }
}
