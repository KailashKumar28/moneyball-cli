//! CRM ticket semantics + the crm.json conformance validator.
//!
//! Owns everything about the crm.json contract (docs/CRM_CONTRACT.md):
//! ticket iteration over both accepted shapes, field extraction, stage
//! semantics, and `check()` - the validator behind `moneyball crm check`
//! that lets any CRM (including custom AI-built ones) iterate against
//! precise errors until its export conforms.

mod check;
pub mod connect;
mod curl;
pub mod fetch;
mod ingest;
pub mod paging;
pub mod presets;
mod refs;
pub mod source;
pub mod status;
pub mod transform;

pub use check::{check, check_with_workspace, CheckReport};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

/// The contract document, printed verbatim by `moneyball crm contract`.
pub const CONTRACT_MD: &str = include_str!("../../../../docs/CRM_CONTRACT.md");

/// Canonical stage names, funnel order. Workspace `crm.stages` may
/// override the recognized set; semantics below always apply.
pub const CANONICAL_STAGES: &[&str] = &[
    "Lost",
    "NonContactable",
    "Contactable",
    "Visit",
    "Revisit",
    "Booking",
];
pub const QUALIFIED_PLUS: &[&str] = &["Contactable", "Visit", "Revisit", "Booking"];
pub const VISIT_PLUS: &[&str] = &["Visit", "Revisit", "Booking"];

/// Funnel milestones for one ticket: (qualified, visited, booked).
/// Booked counts on `funnel == "WON"` regardless of stage.
pub fn milestones(stage: &str, funnel: &str) -> (bool, bool, bool) {
    (
        QUALIFIED_PLUS.contains(&stage),
        VISIT_PLUS.contains(&stage),
        funnel == "WON" || stage == "Booking",
    )
}

/// True when a snapshot has no CRM data at all (crm.json absent or an
/// empty container). Callers must surface this - zero tickets rendered
/// as real zeros silently turns /brief into misinformation.
pub fn is_empty(crm: &Value) -> bool {
    match crm {
        Value::Object(m) => m.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => true,
    }
}

// ---------- ticket iteration ----------

/// Iterate tickets in either accepted crm.json shape: a flat array of
/// tickets (canonical) or the legacy map `{campaign_id: {tickets: [...]}}`.
/// Calls `f(ticket, delivery_epoch)`; delivery defaults to i64::MIN when
/// missing/unparseable so callers' window filters exclude it.
pub fn for_each_ticket<F: FnMut(&serde_json::Map<String, Value>, i64)>(crm: &Value, mut f: F) {
    match crm {
        Value::Object(map) => {
            for (_cid, blob) in map {
                let Some(blob) = blob.as_object() else {
                    continue;
                };
                let Some(tickets) = blob.get("tickets").and_then(|t| t.as_array()) else {
                    continue;
                };
                for t in tickets {
                    let Some(obj) = t.as_object() else { continue };
                    f(obj, delivery_epoch(obj).unwrap_or(i64::MIN));
                }
            }
        }
        Value::Array(arr) => {
            for t in arr {
                let Some(obj) = t.as_object() else { continue };
                f(obj, delivery_epoch(obj).unwrap_or(i64::MIN));
            }
        }
        _ => {}
    }
}

// ---------- field extraction ----------

/// Meta ad id: flat `ad_id` (canonical) or legacy nested `adId.adId`.
pub fn ticket_ad_id(t: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(s) = t.get("ad_id").and_then(|v| v.as_str()) {
        return Some(s.into());
    }
    if let Some(obj) = t.get("adId").and_then(|v| v.as_object()) {
        if let Some(s) = obj.get("adId").and_then(|v| v.as_str()) {
            return Some(s.into());
        }
    }
    None
}

pub fn ticket_stage(t: &serde_json::Map<String, Value>) -> String {
    t.get("stage")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn ticket_funnel(t: &serde_json::Map<String, Value>) -> String {
    t.get("funnel")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn delivery_epoch(t: &serde_json::Map<String, Value>) -> Option<i64> {
    parse_epoch(t.get("delivery")?)
}

/// Epoch seconds from a JSON number, numeric string, or ISO-8601 string.
pub fn parse_epoch(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => {
            if let Ok(i) = s.parse::<i64>() {
                return Some(i);
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.timestamp());
            }
            // Naive ISO - treat as UTC.
            for fmt in &[
                "%Y-%m-%dT%H:%M:%S%z",
                "%Y-%m-%dT%H:%M:%S%.f%z",
                "%Y-%m-%dT%H:%M:%S",
            ] {
                if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
                    return Some(Utc.from_utc_datetime(&dt).timestamp());
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
