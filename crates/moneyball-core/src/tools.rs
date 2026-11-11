//! Tool definitions the agent loop puts on the wire. Each tool is a
//! (name, description, JSON-Schema) triple (codex's approach); the
//! wire-specific serialization lives in llm.rs body builders, and the
//! handlers live in agent/exec.rs (they need only an AppConfig).
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
// live in agent/exec.rs, whose `tools()` is the list that goes on the
// wire. Keep the descriptions specific so the LLM knows when to call each.

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

/// `funnel` - the 7-day performance table for one product, per adset
/// unless asked per campaign or ad. The description must match what the
/// handler actually returns (funnel::table), or the model plans around
/// data it never gets.
pub fn funnel_tool() -> Tool {
    Tool::new(
        "funnel",
        "Return the 7-day table for ONE product, one row per adset (or per campaign / ad when asked): spend, Meta leads (m), cost per lead, CRM leads (l), qualified (q), visits (v), Rs per qualified, L->Q %, kill-table eligibility (kill=true means spend cleared the kill threshold; never recommend killing rows marked immature or learning), and learning status (adsets only). Use this when the user asks why a product is not converting, which adsets to scale or kill, or about lead quality within a product.",
        json!({
            "type": "object",
            "properties": {
                "product": {
                    "type": "string",
                    "description": "Product name exactly as it appears in config.json."
                },
                "by": {
                    "type": "string",
                    "enum": ["campaign", "adset", "ad"],
                    "description": "One row per campaign, adset (the default) or ad."
                }
            },
            "required": ["product"]
        }),
    )
}

/// `diagnose` - the checks for one product before cutting anything
/// (diagnose::text).
pub fn diagnose_tool() -> Tool {
    Tool::new(
        "diagnose",
        "Return four checks for ONE product: where its CRM leads die by campaign (lost before contact = form or audience; unreachable = phone data; reached but not visiting = sales follow-up), which ads are tiring (link CTR, frequency, age), its last 7 days against the days before (spend, leads, cost per lead, qualified, cost per qualified), and what changed in its ad account in the last 7 days (budgets, pauses, new ads, targeting). Call it before recommending a cut, pause or kill, and when the user asks why a product got worse.",
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

/// `recommend` - record a recommendation in the ledger (ledger::submit).
/// The description carries submit's rules, so a refusal is rare and
/// says what to fix.
pub fn recommend_tool() -> Tool {
    let metrics = json!({
        "type": "string",
        "enum": ["spend", "meta_leads", "cpl", "crm_leads", "qualified", "cost_per_qualified", "l_to_q"]
    });
    Tool::new(
        "recommend",
        "Record ONE concrete change you recommend in the ledger, with the number it should move and the date to check it by; moneyball notices when it is applied and settles the prediction from the data. Evidence values are copied from the funnel tool's 7-day table for the same entity and level (metric names map to its columns: meta_leads = m, crm_leads = l, qualified = q, cost_per_qualified = rs_per_q); counts must match exactly, money and ratios within 5%. Prediction targets for spend and counts are per 7 days, like the table. CRM metrics need CRM data. A refusal lists what to fix.",
        json!({
            "type": "object",
            "properties": {
                "product": {"type": "string", "description": "Product name exactly as in config.json."},
                "level": {"type": "string", "enum": ["campaign", "adset", "ad", "product"]},
                "entity_id": {"type": "string", "description": "The id column of the funnel table at this level (ignored for product)."},
                "lever": {"type": "string", "enum": ["budget", "creative", "targeting", "structure", "form", "crm"]},
                "action": {"type": "string", "description": "The change in one sentence, e.g. 'cut the daily budget from Rs 2,000 to Rs 1,700'."},
                "evidence": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {"metric": metrics, "value": {"type": "number"}},
                        "required": ["metric", "value"]
                    }
                },
                "prediction": {
                    "type": "object",
                    "properties": {
                        "metric": metrics,
                        "direction": {"type": "string", "enum": ["up", "down"]},
                        "target": {"type": "number"},
                        "verify_by": {"type": "string", "description": "YYYY-MM-DD, after today and at most 28 days out; give a budget change 3+ days, anything else 7+."}
                    },
                    "required": ["metric", "direction", "target", "verify_by"]
                },
                "budget": {
                    "type": "object",
                    "description": "Required for the budget lever: Rs a day before and after. A move over 20% restarts Meta's learning; set relearn true only when that is intended.",
                    "properties": {
                        "from": {"type": "number"},
                        "to": {"type": "number"},
                        "relearn": {"type": "boolean"}
                    },
                    "required": ["from", "to"]
                }
            },
            "required": ["product", "level", "entity_id", "lever", "action", "evidence", "prediction"]
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
