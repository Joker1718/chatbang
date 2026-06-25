//! Robust JSON-based tool-call protocol (task CB-007).
//!
//! Replaces the previous `<tool_call>...</tool_call>` XML scraping, which
//! relied on `find().unwrap()` and silently dropped any slightly malformed
//! envelope. This module accepts a small family of equivalent delimiters and
//! returns structured `ParseError`s instead of panicking.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use super::error::ToolError;

/// A single tool call extracted from a model response.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

/// Reasons parsing can fail for a single envelope. The whole response may
/// still contain zero, one, or many envelopes — per-envelope failures do not
/// abort the scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// A tool-call block was found but its JSON body could not be parsed.
    MalformedJson { raw: String, reason: String },
    /// The JSON parsed but is missing the `name` field (or it is not a string).
    MissingName { raw: String },
    /// The block delimiter was opened but never closed.
    Unterminated { raw: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::MalformedJson { raw, reason } => {
                write!(f, "malformed JSON in tool call: {} (raw: {:?})", reason, raw)
            }
            ParseError::MissingName { raw } => {
                write!(f, "tool call is missing 'name' field (raw: {:?})", raw)
            }
            ParseError::Unterminated { raw } => {
                write!(f, "tool call block was never closed (raw: {:?})", raw)
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Result of scanning a response: the successfully-parsed calls plus any
/// per-envelope errors (which are also surfaced so the caller can warn the
/// user / model).
#[derive(Debug, Default, Clone)]
pub struct ParsedToolCalls {
    pub calls: Vec<ToolCall>,
    pub errors: Vec<ParseError>,
}

impl ParsedToolCalls {
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty() && self.errors.is_empty()
    }
}

/// All delimiter shapes we accept. The first match wins.
///
///  1. `<tool_call>{...}</tool_call>` — backward compatible with the old XML
///     envelope (still emitted by some models / older prompts).
///  2. ```` ```tool_call\n{...}\n``` ```` — fenced code block, the modern
///     Markdown-friendly form.
///  3. ```` ```json\n{"tool": "...", ...}\n``` ```` — fenced JSON block whose
///     body explicitly identifies itself as a tool call via the `tool` key.
///
/// Matching any of these is intentional: chatbang talks to many backends and
/// each one formats tool calls slightly differently. We round-trip via JSON
/// so we never lose structure.
struct Delimiter {
    /// Captures group 1 = the body between delimiters.
    re: Regex,
    /// How to interpret the body: does the JSON envelope use `name` (legacy)
    /// or `tool` (newer OpenAI-style)?
    style: EnvelopeStyle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvelopeStyle {
    /// Body shape: `{"name": "...", "args": {...}}`
    NameKey,
    /// Body shape: `{"tool": "...", "args": {...}}` or
    /// `{"tool": "...", "parameters": {...}}`
    ToolKey,
}

fn delimiters() -> &'static [Delimiter] {
    static LOCK: OnceLock<Vec<Delimiter>> = OnceLock::new();
    LOCK.get_or_init(|| {
        vec![
            // XML-style (legacy).
            Delimiter {
                re: Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap(),
                style: EnvelopeStyle::NameKey,
            },
            // Fenced ```tool_call ... ```
            Delimiter {
                re: Regex::new(r"(?s)```tool_call\s*\n?(.*?)\n?```").unwrap(),
                style: EnvelopeStyle::NameKey,
            },
            // Fenced ```json ... ``` where the body has a "tool" key.
            Delimiter {
                re: Regex::new(r"(?s)```json\s*\n?(.*?)\n?```").unwrap(),
                style: EnvelopeStyle::ToolKey,
            },
        ]
    })
}

/// Scan `text` and return every tool call found, plus any per-envelope
/// parse errors. Never panics. Never returns `unwrap`-based failures.
pub fn parse_tool_calls(text: &str) -> ParsedToolCalls {
    let mut out = ParsedToolCalls::default();
    for d in delimiters() {
        for caps in d.re.captures_iter(text) {
            let raw = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if raw.is_empty() {
                continue;
            }
            match parse_one(raw, d.style) {
                Ok(c) => out.calls.push(c),
                Err(e) => out.errors.push(e),
            }
        }
    }
    out
}

/// Parse a single envelope body.
fn parse_one(raw: &str, style: EnvelopeStyle) -> Result<ToolCall, ParseError> {
    // Try the expected style first; if that fails, fall back to the other
    // style so a model mixing conventions still works.
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
        ParseError::MalformedJson {
            raw: raw.to_string(),
            reason: e.to_string(),
        }
    })?;

    let obj = v.as_object().ok_or_else(|| ParseError::MalformedJson {
        raw: raw.to_string(),
        reason: "expected JSON object at top level".into(),
    })?;

    // Extract name.
    let name_key = match style {
        EnvelopeStyle::NameKey => "name",
        EnvelopeStyle::ToolKey => "tool",
    };
    let name = obj
        .get(name_key)
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("tool"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError::MissingName {
            raw: raw.to_string(),
        })?
        .to_string();

    // Extract args. Accept `args`, `parameters`, or `input`.
    let args = obj
        .get("args")
        .or_else(|| obj.get("parameters"))
        .or_else(|| obj.get("input"))
        .cloned()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    Ok(ToolCall { name, args })
}

/// Convenience: convert a `ParsedToolCalls` into a `ToolError` list (for
/// callers that want to surface parse failures as structured errors).
pub fn parsed_errors_to_tool_errors(errors: &[ParseError]) -> Vec<ToolError> {
    errors
        .iter()
        .map(|e| ToolError::InvalidArgs(format!("parse: {}", e)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xml_envelope() {
        let s = r#"Here is my plan:
<tool_call>{"name": "run_command", "args": {"command": "ls"}}</tool_call>
Done."#;
        let p = parse_tool_calls(s);
        assert!(p.errors.is_empty());
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.calls[0].name, "run_command");
        assert_eq!(p.calls[0].args["command"], "ls");
    }

    #[test]
    fn parses_fenced_tool_call() {
        let s = "```tool_call\n{\"name\":\"read_file\",\"args\":{\"path\":\"a.txt\"}}\n```";
        let p = parse_tool_calls(s);
        assert!(p.errors.is_empty());
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.calls[0].name, "read_file");
    }

    #[test]
    fn parses_json_fenced_with_tool_key() {
        let s = "```json\n{\"tool\":\"run_command\",\"parameters\":{\"command\":\"pwd\"}}\n```";
        let p = parse_tool_calls(s);
        assert!(p.errors.is_empty());
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.calls[0].name, "run_command");
        assert_eq!(p.calls[0].args["command"], "pwd");
    }

    #[test]
    fn no_tool_call_present() {
        let p = parse_tool_calls("just a normal answer, nothing to call");
        assert!(p.is_empty());
    }

    #[test]
    fn malformed_json_is_structured_error() {
        let s = "<tool_call>{not json}</tool_call>";
        let p = parse_tool_calls(s);
        assert!(p.calls.is_empty());
        assert_eq!(p.errors.len(), 1);
        assert!(matches!(p.errors[0], ParseError::MalformedJson { .. }));
    }

    #[test]
    fn missing_name_is_structured_error() {
        let s = "<tool_call>{\"args\":{}}</tool_call>";
        let p = parse_tool_calls(s);
        assert!(p.calls.is_empty());
        assert_eq!(p.errors.len(), 1);
        assert!(matches!(p.errors[0], ParseError::MissingName { .. }));
    }

    #[test]
    fn multiple_envelopes_in_one_response() {
        let s = r#"
<tool_call>{"name":"a","args":{}}</tool_call>
some text
<tool_call>{"name":"b","args":{}}</tool_call>
"#;
        let p = parse_tool_calls(s);
        assert_eq!(p.calls.len(), 2);
        assert!(p.errors.is_empty());
    }

    #[test]
    fn mixed_good_and_bad_envelopes() {
        let s = r#"
<tool_call>{"name":"a","args":{}}</tool_call>
<tool_call>{bad}</tool_call>
"#;
        let p = parse_tool_calls(s);
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.errors.len(), 1);
    }

    #[test]
    fn args_default_to_empty_object() {
        let s = "<tool_call>{\"name\":\"ping\"}</tool_call>";
        let p = parse_tool_calls(s);
        assert_eq!(p.calls.len(), 1);
        assert!(p.calls[0].args.is_object());
        assert!(p.calls[0].args.as_object().unwrap().is_empty());
    }
}
