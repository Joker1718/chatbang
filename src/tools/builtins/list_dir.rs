//! `list_dir` tool — lists entries in a directory.
//!
//! Async (CB-004): uses `tokio::fs::read_dir`.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tools::error::ToolResult;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct ListDirArgs {
    /// Directory to list. Defaults to the current working directory.
    #[serde(default)]
    pub path: Option<String>,
}

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List files and subdirectories in a directory. Each entry is prefixed \
         with [dir] for directories. Sorted alphabetically."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(ListDirArgs)
    }

    async fn execute(&self, args: &serde_json::Value) -> ToolResult<String> {
        let a: ListDirArgs = serde_json::from_value(args.clone())?;
        let path = a.path.as_deref().unwrap_or(".");
        let mut entries = tokio::fs::read_dir(path).await?;

        let mut lines: Vec<String> = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = if entry
                .file_type()
                .await
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                "[dir] "
            } else {
                "      "
            };
            lines.push(format!("{}{}", kind, name));
        }
        lines.sort();
        if lines.is_empty() {
            Ok("(empty directory)".to_string())
        } else {
            Ok(lines.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lists_current_dir() {
        let t = ListDirTool;
        let out = t
            .execute(&serde_json::json!({ "path": "." }))
            .await
            .unwrap();
        assert!(!out.is_empty());
    }

    #[tokio::test]
    async fn defaults_to_cwd() {
        let t = ListDirTool;
        let out = t.execute(&serde_json::json!({})).await.unwrap();
        assert!(!out.is_empty());
    }

    #[tokio::test]
    async fn empty_dir_reports_empty() {
        let t = ListDirTool;
        let dir = std::env::temp_dir().join("chatbang_empty_dir_test");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let out = t
            .execute(&serde_json::json!({ "path": dir.to_string_lossy() }))
            .await
            .unwrap();
        assert_eq!(out, "(empty directory)");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
