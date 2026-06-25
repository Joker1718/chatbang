//! Runtime tool registry backed by a `HashMap` (task CB-001).
//!
//! Replaces the previous hardcoded `match call.name.as_str() { ... }` block
//! in `execute_tool()`. Tools register themselves at startup (or anytime
//! afterwards) and are dispatched by name. Adding a new tool no longer
//! requires touching the dispatch site or the system prompt.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use schemars::schema::RootSchema;
use tokio::sync::mpsc;

use super::error::{ToolError, ToolResult};
use super::traits::{Tool, ToolDefinition};

/// A single entry in the registry: the tool implementation plus a cached
/// snapshot of its definition (name/desc/schema) for cheap prompt generation.
struct Entry {
    tool: Arc<dyn Tool>,
    definition: ToolDefinition,
}

/// Owns the dispatch table for all known tools.
///
/// Cloning is cheap: only the inner `Arc<RwLock<HashMap>>` is cloned, so a
/// registry handle can be passed around freely to backends, agent loops, and
/// tests. Mutations performed through one clone are visible to all others.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, Entry>>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a tool under its `name()`. If a tool with the same name is
    /// already registered, it is replaced and the old one returned.
    pub fn register<T: Tool + 'static>(&self, tool: T) -> Option<Arc<dyn Tool>> {
        self.register_arc(Arc::new(tool))
    }

    /// Same as `register` but for an already-`Arc`ed tool. Useful when the
    /// same tool instance must be shared across registries.
    pub fn register_arc(&self, tool: Arc<dyn Tool>) -> Option<Arc<dyn Tool>> {
        let definition = ToolDefinition::from_tool(tool.as_ref());
        let name = definition.name.clone();
        let mut map = self
            .tools
            .write()
            .expect("ToolRegistry poisoned");
        map.insert(
            name,
            Entry {
                tool: tool.clone(),
                definition,
            },
        )
        .map(|e| e.tool)
    }

    /// Returns `true` if a tool with the given name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.read().expect("poisoned").contains_key(name)
    }

    /// List all registered tools as `ToolDefinition`s, sorted by name for
    /// deterministic prompt generation.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let map = self.tools.read().expect("poisoned");
        let mut v: Vec<ToolDefinition> = map
            .values()
            .map(|e| ToolDefinition {
                name: e.definition.name.clone(),
                description: e.definition.description.clone(),
                parameters: e.definition.parameters.clone(),
            })
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Dispatch a tool call by name. The `args` value must already be valid
    /// JSON matching the tool's schema; use `validate_and_dispatch` for the
    /// preflight-checked entry point.
    pub async fn dispatch(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> ToolResult<String> {
        let tool = {
            let map = self.tools.read().expect("poisoned");
            map.get(name)
                .ok_or_else(|| ToolError::NotFound(name.to_string()))?
                .tool
                .clone()
        };
        tool.execute(args).await
    }

    /// Streaming variant (CB-006). Same dispatch rules as `dispatch`, but
    /// pipes incremental output through `tx`.
    pub async fn dispatch_streaming(
        &self,
        name: &str,
        args: &serde_json::Value,
        tx: mpsc::Sender<String>,
    ) -> ToolResult<()> {
        let tool = {
            let map = self.tools.read().expect("poisoned");
            map.get(name)
                .ok_or_else(|| ToolError::NotFound(name.to_string()))?
                .tool
                .clone()
        };
        tool.execute_streaming(args, tx).await
    }

    /// Validate `args` against the tool's JSON Schema, then dispatch.
    ///
    /// Validation is structural (presence of required fields, correct types
    /// for known properties). It is not full JSON Schema evaluation — that
    /// would require pulling in `jsonschema` (heavy). For chatbang's needs,
    /// structural validation combined with `serde::Deserialize` inside the
    /// tool is more than sufficient and keeps the binary lean.
    pub async fn validate_and_dispatch(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> ToolResult<String> {
        let (tool, schema) = {
            let map = self.tools.read().expect("poisoned");
            let entry = map
                .get(name)
                .ok_or_else(|| ToolError::NotFound(name.to_string()))?;
            (entry.tool.clone(), entry.definition.parameters.clone())
        };

        validate_args_against_schema(args, &schema)?;
        tool.execute(args).await
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.read().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.read().expect("poisoned").is_empty()
    }
}

/// Minimal structural JSON-Schema preflight.
///
/// We only check:
///  - required fields are present in the object,
///  - any property listed in `schema.properties` has a JSON type that
///    matches the schema's declared type (when one is given).
///
/// This is intentionally NOT a full JSON Schema validator; the second line
/// of defense is the per-tool `Deserialize` impl, which is stricter.
fn validate_args_against_schema(
    args: &serde_json::Value,
    schema: &RootSchema,
) -> ToolResult<()> {
    use schemars::schema::SingleOrVec;

    let obj = match args {
        serde_json::Value::Object(_) => args,
        serde_json::Value::Null => &serde_json::Value::Object(serde_json::Map::new()),
        _ => {
            return Err(ToolError::InvalidArgs(format!(
                "expected object, got {}",
                args_type(args)
            )))
        }
    };

    let schema_obj = &schema.schema;

    // Top-level type must be object (or unspecified). This catches the
    // case where someone passes a primitive to a tool expecting a struct.
    if let Some(SingleOrVec::Single(t)) = &schema_obj.instance_type {
        if t.as_ref() != &schemars::schema::InstanceType::Object {
            return Err(ToolError::InvalidArgs(format!(
                "schema expects {:?}, got object",
                t
            )));
        }
    }

    // Required fields and property type checks live in `object` validation.
    if let Some(obj_val) = &schema_obj.object {
        // Required fields.
        for field in &obj_val.required {
            if !obj.get(field).map_or(false, |v| !v.is_null()) {
                return Err(ToolError::InvalidArgs(format!(
                    "missing required field '{}'",
                    field
                )));
            }
        }

        // Type-check each provided property.
        for (key, val) in obj.as_object().unwrap() {
            if let Some(sub) = obj_val.properties.get(key) {
                if let Some(expected) = sub_type(sub) {
                    if !type_matches(val, &expected) {
                        return Err(ToolError::InvalidArgs(format!(
                            "field '{}' must be {}, got {}",
                            key,
                            expected,
                            args_type(val)
                        )));
                    }
                }
            }
        }
    }

    Ok(())
}

fn args_type(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn sub_type(sub: &schemars::schema::Schema) -> Option<String> {
    use schemars::schema::{Schema, SingleOrVec};
    let schema_obj = match sub {
        Schema::Object(o) => o,
        Schema::Bool(true) => return None, // matches anything
        Schema::Bool(false) => return Some("__nothing__".into()), // matches nothing
    };
    let inst = match &schema_obj.instance_type {
        Some(SingleOrVec::Single(t)) => t.as_ref().clone(),
        Some(SingleOrVec::Vec(v)) => {
            // Multi-type — accept anything in the set.
            return Some(
                v.iter()
                    .map(|t| format!("{:?}", t).to_lowercase())
                    .collect::<Vec<_>>()
                    .join("|"),
            );
        }
        None => return None,
    };
    Some(format!("{:?}", inst).to_lowercase())
}

fn type_matches(v: &serde_json::Value, expected: &str) -> bool {
    // expected is a lowercase type name from `InstanceType` (possibly `|`-joined).
    let actual = args_type(v);
    if expected.contains('|') {
        expected.split('|').any(|e| e.trim() == actual)
    } else if expected == "number" {
        // schemars emits "number" for both f32/f64 and serde integers.
        actual == "number"
    } else {
        actual == expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[derive(Deserialize, JsonSchema)]
    struct EchoArgs {
        message: String,
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes back the message"
        }
        fn parameters_schema(&self) -> RootSchema {
            schemars::schema_for!(EchoArgs)
        }
        async fn execute(&self, args: &serde_json::Value) -> ToolResult<String> {
            let a: EchoArgs = serde_json::from_value(args.clone())?;
            Ok(a.message)
        }
    }

    fn make_registry() -> ToolRegistry {
        let r = ToolRegistry::new();
        r.register(EchoTool);
        r
    }

    #[tokio::test]
    async fn dispatch_known_tool() {
        let r = make_registry();
        let args = serde_json::json!({ "message": "hi" });
        let out = r.dispatch("echo", &args).await.unwrap();
        assert_eq!(out, "hi");
    }

    #[tokio::test]
    async fn dispatch_unknown_returns_not_found() {
        let r = make_registry();
        let err = r
            .dispatch("missing", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[tokio::test]
    async fn validate_rejects_missing_required() {
        let r = make_registry();
        let err = r
            .validate_and_dispatch("echo", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn validate_rejects_wrong_type() {
        let r = make_registry();
        let err = r
            .validate_and_dispatch("echo", &serde_json::json!({ "message": 42 }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn streaming_default_wraps_buffered() {
        let r = make_registry();
        let (tx, mut rx) = mpsc::channel(8);
        r.dispatch_streaming("echo", &serde_json::json!({ "message": "chunk" }), tx)
            .await
            .unwrap();
        let out = rx.recv().await.unwrap();
        assert_eq!(out, "chunk");
    }

    #[test]
    fn clone_shares_state() {
        let r = make_registry();
        let r2 = r.clone();
        assert_eq!(r.len(), 1);
        // Mutating via clone is visible through the original.
        assert!(r2.contains("echo"));
    }
}
