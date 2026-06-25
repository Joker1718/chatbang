//! Built-in tool implementations.
//!
//! Each tool lives in its own module under `src/tools/`. All four implement
//! the `Tool` trait (CB-002), use strongly-typed `Args` structs with
//! `Deserialize + JsonSchema` (CB-003), and run fully async via
//! `tokio::process` / `tokio::fs` (CB-004). `run_command` additionally
//! overrides `execute_streaming` to push stdout line-by-line (CB-006).

pub mod run_command;
pub mod read_file;
pub mod write_file;
pub mod list_dir;

use super::registry::ToolRegistry;

/// Register all built-in tools on the given registry. Call this once at
/// startup — after it returns, the registry is ready to dispatch.
pub fn register_all(reg: &ToolRegistry) {
    reg.register(run_command::RunCommandTool);
    reg.register(read_file::ReadFileTool);
    reg.register(write_file::WriteFileTool);
    reg.register(list_dir::ListDirTool);
}
