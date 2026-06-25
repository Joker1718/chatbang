//! Generic OpenAI-compatible chat completions client.
//!
//! Many providers (Ollama, Groq, Together, OpenAI itself) expose the same
//! `POST /v1/chat/completions` shape. This module implements the shared
//! HTTP+JSON plumbing once; concrete backends just supply a base URL, an
//! API key, and a model name.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Backend, Message, Role};

/// Configuration for an OpenAI-compatible backend.
#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    /// Base URL, no trailing slash. e.g. `https://api.openai.com/v1`,
    /// `http://localhost:11434/v1`, `https://api.groq.com/openai/v1`.
    pub base_url: String,
    /// API key. Empty string for backends that don't require one (Ollama).
    pub api_key: String,
    /// Model identifier, e.g. `gpt-4o-mini`, `llama3.1`, `llama-3.1-8b-instant`.
    pub model: String,
    /// Optional temperature override.
    pub temperature: Option<f32>,
    /// Optional max-tokens override.
    pub max_tokens: Option<u32>,
}

pub struct OpenAiCompatBackend {
    pub cfg: OpenAiCompatConfig,
    pub display_name: &'static str,
    client: reqwest::Client,
}

impl OpenAiCompatBackend {
    pub fn new(cfg: OpenAiCompatConfig, display_name: &'static str) -> Self {
        Self {
            cfg,
            display_name,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("reqwest client"),
        }
    }
}

#[async_trait]
impl Backend for OpenAiCompatBackend {
    fn name(&self) -> &str {
        self.display_name
    }

    async fn chat(&self, messages: &[Message]) -> Result<String> {
        let url = format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/'));
        let body = ChatRequest {
            model: &self.cfg.model,
            messages: messages
                .iter()
                .map(|m| WireMessage {
                    role: match m.role {
                        Role::System => "system",
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::Tool => "tool",
                    },
                    content: &m.content,
                })
                .collect(),
            temperature: self.cfg.temperature,
            max_tokens: self.cfg.max_tokens,
            stream: false,
        };

        let mut req = self.client.post(&url).json(&body);
        if !self.cfg.api_key.is_empty() {
            req = req.bearer_auth(&self.cfg.api_key);
        }

        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "{} returned {}: {}",
                self.display_name,
                status,
                text
            ));
        }

        let parsed: ChatResponse = resp.json().await?;
        parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("{} returned no choices", self.display_name))
    }
}

// ─── Wire types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}
