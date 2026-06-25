//! The tool system.
//!
//! Modules:
//!  - `error`    — `ToolError` enum and conversions (CB-008)
//!  - `traits`   — the `Tool` trait and `ToolDefinition` (CB-002)
//!  - `registry` — `ToolRegistry`, runtime dispatch (CB-001)
//!  - `protocol` — JSON tool-call parsing (CB-007)
//!  - `prompt`   — auto-generate system prompt from registry (CB-010)
//!  - `macros`   — `declare_tool!` / `declare_async_tool!` (CB-009)
//!  - `builtins` — the four built-in tools (CB-003, CB-004, CB-006)

#![allow(unused_imports)]

pub mod error;
pub mod traits;
pub mod registry;
pub mod protocol;
pub mod prompt;
pub mod macros;
pub mod builtins;

pub use error::{ToolError, ToolResult};
pub use protocol::{parse_tool_calls, ParseError, ParsedToolCalls, ToolCall};
pub use prompt::generate_system_prompt;
pub use registry::ToolRegistry;
pub use traits::{Tool, ToolDefinition};
