//! `read_file` tool — reads a UTF-8 file from disk.
//!
//! Async (CB-004): uses `tokio::fs::read_to_string`.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tools::error::{ToolError, ToolResult};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct ReadFileArgs {
    /// Absolute or relative path to the file.
    pub path: String,
    /// If true, return only the first N bytes (default 8 KiB).
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file from disk. Returns the file contents as a \
         single string."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(ReadFileArgs)
    }

    async fn execute(&self, args: &serde_json::Value) -> ToolResult<String> {
        let a: ReadFileArgs = serde_json::from_value(args.clone())?;
        let content = tokio::fs::read_to_string(&a.path).await?;
        if let Some(n) = a.max_bytes {
            if content.len() > n {
                return Ok(content.chars().take(n).collect::<String>());
            }
        }
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_existing_file() {
        let t = ReadFileTool;
        let dir = std::env::temp_dir();
        let p = dir.join("chatbang_read_file_test.txt");
        tokio::fs::write(&p, "hello world").await.unwrap();
        let args = serde_json::json!({ "path": p.to_string_lossy() });
        let out = t.execute(&args).await.unwrap();
        assert_eq!(out, "hello world");
        let _ = tokio::fs::remove_file(&p).await;
    }

    #[tokio::test]
    async fn missing_path_returns_invalid_args() {
        let t = ReadFileTool;
        let err = t.execute(&serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn nonexistent_file_returns_execution_failed() {
        let t = ReadFileTool;
        let err = t
            .execute(&serde_json::json!({ "path": "/nonexistent/nope.txt" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
    }
}
