//! Ollama local backend (CB-011).
//!
//! Ollama exposes an OpenAI-compatible API at `http://localhost:11434/v1`.
//! No API key required. The backend checks whether the Ollama daemon is
//! reachable and, if so, uses the requested model (or `llama3.1` as a
//! safe default). If the daemon is unreachable, `chat()` returns a clear
//! error message instead of silently timing out.

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use super::openai_compat::{OpenAiCompatBackend, OpenAiCompatConfig};
use super::{Backend, Message};

const DEFAULT_BASE_URL: &str = "http://localhost:11434/v1";
const DEFAULT_MODEL: &str = "llama3.1";

pub struct OllamaBackend {
    inner: OpenAiCompatBackend,
}

pub struct OllamaConfig {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            model: None,
            temperature: None,
        }
    }
}

impl OllamaBackend {
    pub fn new(cfg: OllamaConfig) -> Self {
        let cfg = OpenAiCompatConfig {
            base_url: cfg.base_url.unwrap_or_else(|| DEFAULT_BASE_URL.into()),
            api_key: String::new(), // Ollama doesn't require one.
            model: cfg.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            temperature: cfg.temperature,
            max_tokens: None,
        };
        Self {
            inner: OpenAiCompatBackend::new(cfg, "ollama"),
        }
    }

    /// Probe whether the Ollama daemon is reachable. Issues a 2-second
    /// GET to `/api/tags` (the native Ollama list-models endpoint).
    pub async fn is_reachable(&self) -> bool {
        is_daemon_reachable(&self.inner.cfg.base_url).await
    }

    pub fn model(&self) -> &str {
        &self.inner.cfg.model
    }

    pub fn base_url(&self) -> &str {
        &self.inner.cfg.base_url
    }
}

#[async_trait]
impl Backend for OllamaBackend {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn chat(&self, messages: &[Message]) -> Result<String> {
        // Preflight reachability check. We don't cache the result so that
        // starting Ollama mid-session is detected on the next turn.
        if !is_daemon_reachable(&self.inner.cfg.base_url).await {
            return Err(anyhow!(
                "Ollama is not reachable at {}. \
                 Install it from https://ollama.com and run `ollama serve` \
                 in another terminal, then try again.",
                self.inner.cfg.base_url
            ));
        }
        self.inner.chat(messages).await
    }
}

async fn is_daemon_reachable(base_url: &str) -> bool {
    let url = base_url.replace("/v1", "/api/tags");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .get(&url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reachability_returns_false_when_daemon_down() {
        // Point at a port that's almost certainly closed.
        let b = OllamaBackend::new(OllamaConfig {
            base_url: Some("http://127.0.0.1:39999/v1".into()),
            model: None,
            temperature: None,
        });
        let r = b.chat(&[Message::user("hi")]).await;
        assert!(r.is_err());
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("not reachable"));
    }
}
