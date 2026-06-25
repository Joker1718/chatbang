//! `write_file` tool — writes content to a file on disk.
//!
//! Async (CB-004): uses `tokio::fs::write`. Creates parent directories if
//! they don't exist.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tools::error::ToolResult;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct WriteFileArgs {
    /// Absolute or relative path to the file. Parent directories are created
    /// automatically.
    pub path: String,
    /// The content to write. Overwrites the file if it already exists.
    pub content: String,
    /// If true, append to the file instead of overwriting. Default false.
    #[serde(default)]
    pub append: bool,
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file on disk. Creates parent directories if \
         needed. Overwrites by default; pass \"append\": true to append."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(WriteFileArgs)
    }

    async fn execute(&self, args: &serde_json::Value) -> ToolResult<String> {
        let a: WriteFileArgs = serde_json::from_value(args.clone())?;

        // Ensure parent dir exists.
        if let Some(parent) = std::path::Path::new(&a.path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        let bytes = a.content.len();
        if a.append {
            // Use OpenOptions for append.
            use tokio::io::AsyncWriteExt;
            let mut f = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&a.path)
                .await?;
            f.write_all(a.content.as_bytes()).await?;
            f.flush().await?;
        } else {
            tokio::fs::write(&a.path, a.content.as_bytes()).await?;
        }

        Ok(format!("Wrote {} bytes to {}", bytes, a.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_and_overwrites() {
        let t = WriteFileTool;
        let p = std::env::temp_dir().join("chatbang_write_test.txt");
        let _ = tokio::fs::remove_file(&p).await;

        let args = serde_json::json!({
            "path": p.to_string_lossy(),
            "content": "first"
        });
        t.execute(&args).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&p).await.unwrap(),
            "first"
        );

        let args = serde_json::json!({
            "path": p.to_string_lossy(),
            "content": "second"
        });
        t.execute(&args).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&p).await.unwrap(),
            "second"
        );

        let _ = tokio::fs::remove_file(&p).await;
    }

    #[tokio::test]
    async fn appends_when_requested() {
        let t = WriteFileTool;
        let p = std::env::temp_dir().join("chatbang_append_test.txt");
        let _ = tokio::fs::remove_file(&p).await;

        let a1 = serde_json::json!({
            "path": p.to_string_lossy(),
            "content": "first",
            "append": true
        });
        t.execute(&a1).await.unwrap();

        let a2 = serde_json::json!({
            "path": p.to_string_lossy(),
            "content": "second",
            "append": true
        });
        t.execute(&a2).await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&p).await.unwrap(),
            "firstsecond"
        );
        let _ = tokio::fs::remove_file(&p).await;
    }

    #[tokio::test]
    async fn creates_parent_dirs() {
        let t = WriteFileTool;
        let dir = std::env::temp_dir().join("chatbang_subdir_test");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let p = dir.join("nested").join("file.txt");
        let args = serde_json::json!({
            "path": p.to_string_lossy(),
            "content": "x"
        });
        t.execute(&args).await.unwrap();
        assert!(p.exists());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
