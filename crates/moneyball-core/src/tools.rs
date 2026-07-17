//! Tool definitions the agent loop puts on the wire. Each tool is a
//! (name, description, JSON-Schema) triple (codex's approach); the
//! wire-specific serialization lives in llm.rs body builders, and the
//! handlers live in moneyball-tui (they need App + snapshot access).
//! Only tools with real handlers may be defined here - descriptions
//! are promises to the model.

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


}
