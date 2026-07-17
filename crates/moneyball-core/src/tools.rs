//! Tool-calling protocol types + tool registry. Mirrors openai/codex's
//! approach: each tool is a (name, description, JSON-Schema) triple;
//! the LLM client exposes a `complete_with_tools` method that returns
//! either a final assistant message or a list of tool calls. The agent
//! loop in moneyball-tui executes the tool calls and feeds the results
//! back as a second turn.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A tool the LLM can invoke. `parameters` is a JSON Schema (subset:
/// type=object, properties, required) that describes the input shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl Tool {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// A tool call requested by the LLM. The agent loop dispatches on
/// `name` and feeds `arguments` (already JSON-parsed) to the handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned id. Echoed back in the result so the LLM
    /// knows which call the result corresponds to.
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// A tool's response, fed back to the LLM as the next turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            is_error: false,
        }
    }
    pub fn err(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            is_error: true,
        }
    }
}

/// What the LLM returned for a single turn. Either text (a final
/// answer) or one-or-more tool calls (the agent must run them and
/// call back).
#[derive(Debug, Clone)]
pub enum Completion {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

impl Completion {
    pub fn is_tool_call(&self) -> bool {
        matches!(self, Completion::ToolCalls(_))
    }
}

// ---------- built-in tool definitions ----------
//
// These describe the data-side capabilities of moneyball. The handlers
// live in moneyball-tui (they need App + snapshot access). Keep the
// descriptions specific so the LLM knows when to call each.

/// `brief` - load the 7-day portfolio snapshot and return a markdown
/// summary table plus the feasibility math. Use this whenever the user
/// asks about overall performance, "what's happening", or wants to
/// compare products.
pub fn brief_tool() -> Tool {
    Tool::new(
        "brief",
        "Return the 7-day portfolio summary: per-product spend / leads / qualified / L->Q / goal / gap, plus portfolio feasibility math (current Rs/qualified, best-observed Rs/qualified, required spend to hit goal). Use this when the user asks about overall performance, asks 'what is my best/worst product', or wants a feasibility check.",
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    )
}

/// `funnel` - the per-adset 7-day performance table for one product.
/// The description must match what the handler actually returns
/// (funnel::table), or the model plans around data it never gets.
pub fn funnel_tool() -> Tool {
    Tool::new(
        "funnel",
        "Return the per-adset 7-day table for ONE product: spend, Meta leads (m), cost per lead, CRM leads (l), qualified (q), visits (v), Rs per qualified, L->Q %, kill-table eligibility (kill=true means spend cleared the kill threshold; never recommend killing rows marked immature or learning), and learning status. Use this when the user asks why a product is not converting, which adsets to scale or kill, or about lead quality within a product.",
        json!({
            "type": "object",
            "properties": {
                "product": {
                    "type": "string",
                    "description": "Product name exactly as it appears in config.json."
                }
            },
            "required": ["product"]
        }),
    )
}

// ---------- wire-protocol tool serialization ----------
//
// Each provider wire format wants tools in a slightly different shape.
// `tools_for(wire)` returns the JSON value the provider expects.

/// OpenAI Responses / ChatCompletions tool schema.
fn openai_tool(t: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        }
    })
}

/// Anthropic Messages tool schema.
fn anthropic_tool(t: &Tool) -> Value {
    json!({
        "name": t.name,
        "description": t.description,
        "input_schema": t.parameters,
    })
}

/// Convert the registry into the provider's expected format.
pub fn tools_payload(wire: WireApi, tools: &[Tool]) -> Value {
    match wire {
        WireApi::Responses | WireApi::ChatCompletions => {
            Value::Array(tools.iter().map(openai_tool).collect())
        }
        WireApi::Messages => Value::Array(tools.iter().map(anthropic_tool).collect()),
    }
}

use crate::provider::WireApi;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_tool_has_no_required_args() {
        let t = brief_tool();
        assert_eq!(t.name, "brief");
        assert_eq!(t.parameters["type"], "object");
        assert_eq!(
            t.parameters["additionalProperties"],
            serde_json::Value::Bool(false)
        );
    }

    #[test]
    fn funnel_tool_requires_product_arg() {
        let t = funnel_tool();
        let required = t.parameters["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "product"));
    }

    #[test]
    fn tools_payload_openai_uses_function_type() {
        let v = tools_payload(WireApi::ChatCompletions, &[brief_tool()]);
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[0]["function"]["name"], "brief");
    }

    #[test]
    fn tools_payload_anthropic_uses_input_schema() {
        let v = tools_payload(WireApi::Messages, &[brief_tool()]);
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["name"], "brief");
        assert!(arr[0].get("input_schema").is_some());
    }
}
