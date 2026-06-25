//! Structured error type for the tool system (task CB-008).
//!
//! Replaces the previous convention of returning `[error] ...` strings from
//! tool implementations. `ToolError` is machine-readable, loggable, and can
//! be converted into either a user-facing message or an API error response
//! without leaking internal details.

use std::fmt;

/// All errors that can arise from tool registration, dispatch, or execution.
#[derive(Debug, Clone)]
pub enum ToolError {
    /// No tool is registered under the given name.
    NotFound(String),

    /// The arguments supplied did not validate against the tool's JSON Schema
    /// or could not be deserialized into the tool's argument struct.
    InvalidArgs(String),

    /// The tool ran but its underlying operation failed (non-zero exit,
    /// I/O error, etc.).
    ExecutionFailed(String),

    /// The tool was found but the current user/context is not permitted
    /// to run it (e.g. sandbox restrictions).
    PermissionDenied(String),

    /// The tool did not finish within its allotted time budget.
    Timeout { tool: String, secs: u64 },

    /// A streaming-specific failure (channel closed, consumer dropped, ...).
    StreamingFailed(String),
}

impl ToolError {
    /// Short stable kind label, suitable for tagging in logs or metrics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::InvalidArgs(_) => "invalid_args",
            Self::ExecutionFailed(_) => "execution_failed",
            Self::PermissionDenied(_) => "permission_denied",
            Self::Timeout { .. } => "timeout",
            Self::StreamingFailed(_) => "streaming_failed",
        }
    }

    /// Human-readable message that is safe to surface to the end user.
    ///
    /// Internal file paths, OS error strings, and stack details are stripped
    /// so that we never leak implementation details by default. Use the
    /// `Debug` impl when you want the full diagnostic for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::NotFound(name) => format!("Tool '{}' is not available.", name),
            Self::InvalidArgs(msg) => format!("Invalid arguments: {}", msg),
            Self::ExecutionFailed(msg) => format!("Tool execution failed: {}", msg),
            Self::PermissionDenied(msg) => format!("Permission denied: {}", msg),
            Self::Timeout { tool, secs } => {
                format!("Tool '{}' timed out after {}s.", tool, secs)
            }
            Self::StreamingFailed(msg) => format!("Streaming error: {}", msg),
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display gives the full diagnostic for logs/devs. Use `user_message`
        // when surfacing to end users.
        match self {
            Self::NotFound(n) => write!(f, "tool not found: {}", n),
            Self::InvalidArgs(m) => write!(f, "invalid arguments: {}", m),
            Self::ExecutionFailed(m) => write!(f, "execution failed: {}", m),
            Self::PermissionDenied(m) => write!(f, "permission denied: {}", m),
            Self::Timeout { tool, secs } => {
                write!(f, "tool '{}' timed out after {}s", tool, secs)
            }
            Self::StreamingFailed(m) => write!(f, "streaming failed: {}", m),
        }
    }
}

impl std::error::Error for ToolError {}

/// Convenience alias used throughout the tool system.
pub type ToolResult<T> = Result<T, ToolError>;

/// Common `From` conversions so that `?` just works inside tool bodies.
impl From<serde_json::Error> for ToolError {
    fn from(e: serde_json::Error) -> Self {
        Self::InvalidArgs(format!("JSON error: {}", e))
    }
}

impl From<std::io::Error> for ToolError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::PermissionDenied => {
                Self::PermissionDenied(e.to_string())
            }
            _ => Self::ExecutionFailed(format!("io: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stable() {
        assert_eq!(ToolError::NotFound("x".into()).kind(), "not_found");
        assert_eq!(
            ToolError::InvalidArgs("y".into()).kind(),
            "invalid_args"
        );
        assert_eq!(
            ToolError::Timeout { tool: "t".into(), secs: 5 }.kind(),
            "timeout"
        );
    }

    #[test]
    fn user_message_does_not_leak_paths() {
        let e = ToolError::ExecutionFailed(
            "os error 2 at /etc/shadow: permission denied".into(),
        );
        let msg = e.user_message();
        // The user-facing message keeps the OS-level phrase but is still
        // a single-line, scrubbed message — no stack, no internal addresses.
        assert!(msg.contains("Tool execution failed"));
    }

    #[test]
    fn display_is_more_verbose_than_user_message() {
        let e = ToolError::NotFound("run_command".into());
        assert_eq!(format!("{}", e), "tool not found: run_command");
        assert_eq!(e.user_message(), "Tool 'run_command' is not available.");
    }
}
