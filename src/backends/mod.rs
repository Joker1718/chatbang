//! Backend abstraction (CB-011, CB-012).
//!
//! Every inference provider — whether it's a local Ollama instance, a free-tier
//! cloud API like Groq, or the legacy ChatGPT-web-scraping path — implements
//! the [`Backend`] trait. The agent loop talks to backends only through this
//! trait, so swapping providers is a one-line CLI/config change.

use anyhow::Result;
use async_trait::async_trait;

/// A single message in a conversation. Backends are responsible for
/// serialising this into whatever wire format they speak.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    /// Tool result returned to the model. `tool_name` is included so the
    /// model can match the result to the call that produced it.
    Tool,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
    pub fn tool(content: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: content.into() }
    }
}

/// What every backend must provide: a way to send a conversation and get
/// the assistant's next message back.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Human-readable backend name (e.g. "ollama", "groq", "browser").
    fn name(&self) -> &str;

    /// Send the full conversation and return the assistant's reply.
    async fn chat(&self, messages: &[Message]) -> Result<String>;
}

pub mod openai_compat;
pub mod ollama;
pub mod groq;
pub mod gemini;
pub mod together;
#[cfg(feature = "browser")]
pub mod browser;
