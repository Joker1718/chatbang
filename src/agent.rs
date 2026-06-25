//! The agentic tool-use loop.
//!
//! Given a [`Backend`] and a [`ToolRegistry`], runs the interactive REPL.
//! Each user turn is dispatched to the backend; if the response contains
//! tool calls (CB-007), they are executed via the registry (CB-001) and
//! the results are sent back to the backend as a follow-up message. The
//! loop continues until the model produces a response with no tool calls.

use anyhow::Result;
use std::io::{self, BufRead, Write};

use crate::backends::{Backend, Message, Role};
use crate::tools::error::ToolError;
use crate::tools::protocol::parse_tool_calls;
use crate::tools::registry::ToolRegistry;
use crate::tools::ToolCall;

/// Configuration for the chat REPL.
pub struct AgentConfig {
    /// When true, the tool-use loop is enabled. When false, the backend is
    /// treated as a plain chat — tool-call envelopes in responses are
    /// printed verbatim and never executed.
    pub agent: bool,
    /// Cap on how many tool-use round trips are allowed in a single user
    /// turn before we bail out (prevents infinite loops).
    pub max_iterations: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            agent: true,
            max_iterations: 10,
        }
    }
}

/// Run the REPL. Reads lines from stdin, dispatches to the backend, executes
/// any tool calls, prints the final answer, then waits for the next line.
pub async fn run(
    backend: &dyn Backend,
    registry: &ToolRegistry,
    cfg: &AgentConfig,
) -> Result<()> {
    let skin = termimad::MadSkin::default();

    if cfg.agent {
        let defs = registry.definitions();
        let names: Vec<String> = defs.iter().map(|d| d.name.clone()).collect();
        eprintln!("Agent mode ON  —  tools: {}", names.join(", "));
        eprintln!("Use --no-agent to disable.\n");
    }

    let stdin = io::stdin();
    let mut first_message = true;
    let system_prompt_text = if cfg.agent {
        Some(crate::tools::generate_system_prompt(registry))
    } else {
        None
    };

    print!("> ");
    io::stdout().flush()?;

    for line in stdin.lock().lines() {
        let line = line?;
        let raw = line.trim().to_string();
        if raw.is_empty() {
            print!("> ");
            io::stdout().flush()?;
            continue;
        }

        // Build the first user message, prepending the system prompt.
        let first_user = if first_message {
            first_message = false;
            if let Some(p) = &system_prompt_text {
                format!("{}{}", p, raw)
            } else if cfg.agent {
                raw.clone()
            } else {
                format!("{} (Make an answer in less than 5 lines.)", raw)
            }
        } else {
            raw.clone()
        };

        let mut messages: Vec<Message> = vec![Message::user(first_user)];

        eprintln!("[Thinking...]\n");

        // Inner tool-use loop.
        let mut iterations = 0;
        loop {
            iterations += 1;
            if iterations > cfg.max_iterations {
                eprintln!("[agent] hit max iterations ({}), stopping.", cfg.max_iterations);
                break;
            }

            let response = match backend.chat(&messages).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error: {:#}", e);
                    break;
                }
            };

            // Always echo the assistant's raw response into the conversation
            // history so the backend sees its own prior turn.
            messages.push(Message {
                role: Role::Assistant,
                content: response.clone(),
            });

            if !cfg.agent {
                skin.print_text(&response);
                println!();
                break;
            }

            let parsed = parse_tool_calls(&response);
            for err in &parsed.errors {
                eprintln!("[agent] parse warning: {}", err);
            }

            if parsed.calls.is_empty() {
                skin.print_text(&response);
                println!();
                break;
            }

            // Execute each tool call and append the results as a single
            // tool-role message.
            let mut tool_results = String::new();
            for call in &parsed.calls {
                eprintln!("[tool] {} {}", call.name, call.args);
                let out = execute_tool_call(registry, call).await;
                eprintln!("[tool] -> {}", summarize(&out));
                tool_results.push_str(&format!(
                    "<tool_result name=\"{}\">\n{}\n</tool_result>\n",
                    call.name, out
                ));
            }
            eprintln!(
                "[agent] executed {} tool(s), sending results back...\n",
                parsed.calls.len()
            );

            messages.push(Message::tool(tool_results));
        }

        print!("> ");
        io::stdout().flush()?;
    }

    Ok(())
}

/// Execute a single tool call, translating `ToolError` into a user-readable
/// string that gets fed back to the model. We deliberately do NOT surface
/// internal details — `user_message()` is the scrubbed version.
async fn execute_tool_call(registry: &ToolRegistry, call: &ToolCall) -> String {
    match registry
        .validate_and_dispatch(&call.name, &call.args)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            let kind = e.kind();
            let msg = e.user_message();
            format!("[error:{}] {}", kind, msg)
        }
    }
}

/// One-line preview of a tool's output for the agent log.
fn summarize(s: &str) -> String {
    let first_line = s.lines().next().unwrap_or("");
    let truncated = if first_line.len() > 80 {
        format!("{}...", &first_line[..77])
    } else {
        first_line.to_string()
    };
    truncated.replace('\n', " ")
}

#[allow(dead_code)]
fn unused_suppress(_e: &ToolError) {}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// A backend that returns a canned sequence of responses, one per call.
    struct ScriptedBackend {
        responses: Arc<Mutex<std::collections::VecDeque<String>>>,
    }

    #[async_trait]
    impl Backend for ScriptedBackend {
        fn name(&self) -> &str {
            "scripted"
        }
        async fn chat(&self, _messages: &[Message]) -> Result<String> {
            let mut q = self.responses.lock().unwrap();
            if q.is_empty() {
                Ok("done".into())
            } else {
                Ok(q.pop_front().unwrap())
            }
        }
    }

    fn make_registry() -> ToolRegistry {
        let r = ToolRegistry::new();
        crate::tools::builtins::register_all(&r);
        r
    }

    #[tokio::test]
    async fn no_tool_call_terminates_immediately() {
        let b = ScriptedBackend {
            responses: Arc::new(Mutex::new(vec!["hello world".into()].into())),
        };
        // We can't easily test stdin-driven REPL, but we can test the inner
        // execute_tool_call helper directly.
        let reg = make_registry();
        let call = ToolCall {
            name: "list_dir".into(),
            args: serde_json::json!({ "path": "." }),
        };
        let out = execute_tool_call(&reg, &call).await;
        assert!(!out.starts_with("[error"));
    }

    #[tokio::test]
    async fn unknown_tool_returns_user_friendly_error() {
        let reg = make_registry();
        let call = ToolCall {
            name: "nonexistent".into(),
            args: serde_json::json!({}),
        };
        let out = execute_tool_call(&reg, &call).await;
        assert!(out.starts_with("[error:not_found]"));
    }

    #[tokio::test]
    async fn invalid_args_returns_user_friendly_error() {
        let reg = make_registry();
        let call = ToolCall {
            name: "run_command".into(),
            args: serde_json::json!({}),
        };
        let out = execute_tool_call(&reg, &call).await;
        assert!(out.starts_with("[error:invalid_args]"));
    }
}
