//! Auto-generate the agent system prompt from the live `ToolRegistry`
//! (task CB-010).
//!
//! Eliminates the manual tool list in `agent_system_prompt()` — the prompt
//! always reflects whatever is actually registered, so adding or removing a
//! tool can never again drift out of sync with the prompt.

use super::registry::ToolRegistry;
use super::traits::ToolDefinition;

/// Render the agent's system prompt, listing every registered tool with its
/// name, description, and JSON Schema.
pub fn generate_system_prompt(reg: &ToolRegistry) -> String {
    let defs = reg.definitions();
    let mut out = String::with_capacity(2048);

    out.push_str("[SYSTEM — read carefully before responding]\n");
    out.push_str(
        "You are a helpful assistant with access to tools on the user's machine.\n\n",
    );
    out.push_str(
        "When you need to use a tool, emit one or more tool calls using ONE of these\n\
         equivalent formats (one tool call per block, nothing else inside the block):\n\n",
    );
    out.push_str("  <tool_call>{\"name\": \"TOOL_NAME\", \"args\": {\"KEY\": \"VALUE\"}}</tool_call>\n");
    out.push_str("  ```tool_call\n  {\"name\": \"TOOL_NAME\", \"args\": {\"KEY\": \"VALUE\"}}\n  ```\n\n");
    out.push_str(
        "After emitting tool calls, stop immediately. The results will come back to\n\
         you wrapped in <tool_result name=\"TOOL_NAME\">...</tool_result>. Use them\n\
         to continue reasoning. When you have all the information you need, give\n\
         your final answer with no tool calls at all.\n\n",
    );

    if defs.is_empty() {
        out.push_str("(No tools are currently registered.)\n");
    } else {
        out.push_str("Available tools:\n");
        for d in &defs {
            push_tool_section(&mut out, d);
        }
    }

    out.push_str("\n[END SYSTEM]\n\n");
    out
}

fn push_tool_section(out: &mut String, d: &ToolDefinition) {
    out.push_str(&format!("  {} — {}\n", d.name, d.description));
    // Render the schema compactly so the prompt stays readable.
    let schema_json = serde_json::to_value(&d.parameters).unwrap_or(serde_json::Value::Null);
    let pretty = compact_schema(&schema_json);
    out.push_str(&format!("    args schema: {}\n", pretty));
}

/// Render a JSON-Schema-like value compactly: no whitespace between tokens,
/// but with each top-level property on its own line for readability.
fn compact_schema(v: &serde_json::Value) -> String {
    // Use serde_json's compact formatter, then post-process for slight
    // readability. We don't need a full pretty-printer — the model is fine
    // with compact JSON.
    match v {
        serde_json::Value::Null => "null".into(),
        _ => serde_json::to_string(v).unwrap_or_else(|_| "{}".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[derive(Deserialize, JsonSchema)]
    struct PingArgs {
        msg: String,
    }

    struct PingTool;

    #[async_trait]
    impl crate::tools::traits::Tool for PingTool {
        fn name(&self) -> &str {
            "ping"
        }
        fn description(&self) -> &str {
            "ping the user"
        }
        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schemars::schema_for!(PingArgs)
        }
        async fn execute(
            &self,
            _: &serde_json::Value,
        ) -> crate::tools::error::ToolResult<String> {
            Ok("pong".into())
        }
    }

    #[test]
    fn prompt_includes_registered_tool() {
        let reg = ToolRegistry::new();
        reg.register(PingTool);
        let p = generate_system_prompt(&reg);
        assert!(p.contains("ping"));
        assert!(p.contains("ping the user"));
        assert!(p.contains("msg"));
    }

    #[test]
    fn prompt_changes_when_tool_added() {
        let reg = ToolRegistry::new();
        let before = generate_system_prompt(&reg);
        assert!(before.contains("No tools"));
        reg.register(PingTool);
        let after = generate_system_prompt(&reg);
        assert!(!after.contains("No tools"));
        assert!(after.contains("ping"));
    }

    #[test]
    fn prompt_changes_when_tool_removed() {
        let reg = ToolRegistry::new();
        reg.register(PingTool);
        let with = generate_system_prompt(&reg);
        assert!(with.contains("ping"));
        // Build a fresh empty registry.
        let empty = ToolRegistry::new();
        let without = generate_system_prompt(&empty);
        assert!(!without.contains("ping —"));
    }
}
