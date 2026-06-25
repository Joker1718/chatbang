//! `run_command` tool — executes a shell command via the host's default
//! shell interpreter.
//!
//! Cross-platform (CB-005): uses `cmd /C` on Windows and `sh -c` on Unix,
//! via the [`crate::platform`] helpers.
//!
//! Async (CB-004): uses `tokio::process::Command` so long-running commands
//! do not block the runtime.
//!
//! Streaming (CB-006): overrides `execute_streaming` to push stdout lines
//! as they arrive, instead of waiting for the process to exit.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::platform::Shell;
use crate::tools::error::{ToolError, ToolResult};
use crate::tools::traits::Tool;

/// Typed arguments for `run_command` (CB-003).
#[derive(Deserialize, JsonSchema)]
pub struct RunCommandArgs {
    /// The shell command to execute. Passed verbatim to the host shell.
    pub command: String,
    /// Optional working directory. Defaults to the current process dir.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional timeout in seconds. Defaults to 120.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

pub struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Run a shell command on the user's machine. Uses the host's default \
         shell (cmd on Windows, sh on Linux/macOS). Returns combined stdout \
         and stderr."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(RunCommandArgs)
    }

    async fn execute(&self, args: &serde_json::Value) -> ToolResult<String> {
        let a: RunCommandArgs = serde_json::from_value(args.clone())?;

        let shell = Shell::host();
        let mut cmd = tokio::process::Command::new(shell.binary());
        cmd.arg(shell.command_flag()).arg(&a.command);

        if let Some(cwd) = &a.cwd {
            cmd.current_dir(cwd);
        }

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let timeout_secs = a.timeout_secs.unwrap_or(120);

        let child = cmd
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("spawn: {}", e)))?;

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| ToolError::Timeout {
            tool: "run_command".into(),
            secs: timeout_secs,
        })?
        .map_err(|e| ToolError::ExecutionFailed(format!("wait: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut result = stdout.trim().to_string();
        if !stderr.trim().is_empty() {
            if !result.is_empty() {
                result.push_str("\n[stderr] ");
            } else {
                result.push_str("[stderr] ");
            }
            result.push_str(stderr.trim());
        }
        if result.is_empty() {
            Ok("(no output)".to_string())
        } else {
            Ok(result)
        }
    }

    /// Stream stdout line-by-line as it arrives (CB-006).
    async fn execute_streaming(
        &self,
        args: &serde_json::Value,
        tx: mpsc::Sender<String>,
    ) -> ToolResult<()> {
        let a: RunCommandArgs = serde_json::from_value(args.clone())?;

        let shell = Shell::host();
        let mut cmd = tokio::process::Command::new(shell.binary());
        cmd.arg(shell.command_flag()).arg(&a.command);

        if let Some(cwd) = &a.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let timeout_secs = a.timeout_secs.unwrap_or(120);
        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("spawn: {}", e)))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("no stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("no stderr".into()))?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

        loop {
            tokio::select! {
                biased;

                _ = tokio::time::sleep_until(deadline) => {
                    let _ = child.kill().await;
                    return Err(ToolError::Timeout {
                        tool: "run_command".into(),
                        secs: timeout_secs,
                    });
                }

                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            if tx.send(l).await.is_err() {
                                // Consumer dropped — kill the child and exit.
                                let _ = child.kill().await;
                                return Ok(());
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = child.kill().await;
                            return Err(ToolError::ExecutionFailed(
                                format!("read stdout: {}", e)
                            ));
                        }
                    }
                }

                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            let prefixed = format!("[stderr] {}", l);
                            if tx.send(prefixed).await.is_err() {
                                let _ = child.kill().await;
                                return Ok(());
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let _ = child.kill().await;
                            return Err(ToolError::ExecutionFailed(
                                format!("read stderr: {}", e)
                            ));
                        }
                    }
                }
            }
        }

        let status = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            child.wait(),
        )
        .await
        .map_err(|_| ToolError::Timeout {
            tool: "run_command".into(),
            secs: 5,
        })?
        .map_err(|e| ToolError::ExecutionFailed(format!("wait: {}", e)))?;

        if !status.success() {
            let _ = tx
                .send(format!("[exit code: {}]", status.code().unwrap_or(-1)))
                .await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn executes_simple_command() {
        let t = RunCommandTool;
        let args = if cfg!(target_os = "windows") {
            serde_json::json!({ "command": "echo hello" })
        } else {
            serde_json::json!({ "command": "echo hello" })
        };
        let out = t.execute(&args).await.unwrap();
        assert!(out.contains("hello"));
    }

    #[tokio::test]
    async fn missing_command_arg_returns_invalid_args() {
        let t = RunCommandTool;
        let err = t.execute(&serde_json::json!({})).await.unwrap_err();
        // serde_json error gets converted into InvalidArgs via the From impl.
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn streaming_emits_lines() {
        let t = RunCommandTool;
        let cmd = if cfg!(target_os = "windows") {
            "echo line1 && echo line2"
        } else {
            "echo line1; echo line2"
        };
        let (tx, mut rx) = mpsc::channel(8);
        t.execute_streaming(&serde_json::json!({ "command": cmd }), tx)
            .await
            .unwrap();
        let mut lines = Vec::new();
        while let Some(l) = rx.recv().await {
            lines.push(l);
        }
        assert!(lines.iter().any(|l| l.contains("line1")));
        assert!(lines.iter().any(|l| l.contains("line2")));
    }
}
