//! CRM ticket semantics + the crm.json conformance validator.
//!
//! Owns everything about the crm.json contract (docs/CRM_CONTRACT.md):
//! ticket iteration over both accepted shapes, field extraction, stage
//! semantics, and `check()` - the validator behind `moneyball crm check`
//! that lets any CRM (including custom AI-built ones) iterate against
//! precise errors until its export conforms.

pub mod connect;
pub mod fetch;
pub mod source;

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::snapshot::Snapshot;

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

// ---------- validator ----------

#[derive(Debug, Default)]
pub struct CheckReport {
    pub tickets: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub info: Vec<String>,
}

impl CheckReport {
    pub fn passed(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Validate a parsed crm.json against the contract. `stages` is the
/// recognized stage set (pass `&[]` to use CANONICAL_STAGES); `snap`,
/// when given, enables the ad_id join-rate check against `ads_daily`.
pub fn check(crm: &Value, stages: &[String], snap: Option<&Snapshot>) -> CheckReport {
    let mut r = CheckReport::default();

    let tickets = match collect_tickets(crm, &mut r) {
        Some(t) => t,
        None => return r,
    };
    r.tickets = tickets.len();
    if tickets.is_empty() {
        r.warnings.push("0 tickets - empty export".into());
        return r;
    }

    check_required_fields(&tickets, &mut r);
    check_stages(&tickets, stages, &mut r);
    check_delivery_range(&tickets, &mut r);
    if let Some(snap) = snap {
        check_join_rate(&tickets, snap, &mut r);
    } else {
        r.info
            .push("no snapshot in workspace - ad_id join-rate check skipped".into());
    }
    r
}

/// Flatten either accepted shape into a ticket list, reporting shape
/// errors. `None` means the shape is unrecognized (already reported).
fn collect_tickets<'a>(
    crm: &'a Value,
    r: &mut CheckReport,
) -> Option<Vec<&'a serde_json::Map<String, Value>>> {
    match crm {
        Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, t) in arr.iter().enumerate() {
                match t.as_object() {
                    Some(obj) => out.push(obj),
                    None => {
                        push_sampled(&mut r.errors, format!("ticket[{}] is not a JSON object", i))
                    }
                }
            }
            Some(out)
        }
        Value::Object(map) => {
            let mut out = Vec::new();
            for (cid, blob) in map {
                let tickets = blob
                    .as_object()
                    .and_then(|b| b.get("tickets"))
                    .and_then(|t| t.as_array());
                let Some(tickets) = tickets else {
                    push_sampled(
                        &mut r.errors,
                        format!("legacy map entry \"{}\" has no tickets array", cid),
                    );
                    continue;
                };
                for (i, t) in tickets.iter().enumerate() {
                    match t.as_object() {
                        Some(obj) => out.push(obj),
                        None => push_sampled(
                            &mut r.errors,
                            format!("\"{}\".tickets[{}] is not a JSON object", cid, i),
                        ),
                    }
                }
            }
            r.info
                .push("legacy campaign-map shape (flat array preferred)".into());
            Some(out)
        }
        _ => {
            r.errors.push(
                "crm.json must be a JSON array of ticket objects (see moneyball crm contract)"
                    .into(),
            );
            None
        }
    }
}

const SAMPLE_CAP: usize = 5;

/// Keep error lists readable: first SAMPLE_CAP concrete rows, then a count.
fn push_sampled(list: &mut Vec<String>, msg: String) {
    if list.len() < SAMPLE_CAP {
        list.push(msg);
    } else if list.len() == SAMPLE_CAP {
        list.push("... more of the same (fix the above and re-run)".into());
    }
}

fn check_required_fields(tickets: &[&serde_json::Map<String, Value>], r: &mut CheckReport) {
    let (mut no_ad, mut no_stage, mut no_delivery) = (0usize, 0usize, 0usize);
    let mut samples: Vec<String> = Vec::new();
    for (i, t) in tickets.iter().enumerate() {
        if ticket_ad_id(t).is_none_or(|s| s.trim().is_empty()) {
            no_ad += 1;
            push_sampled(
                &mut samples,
                format!("ticket[{}]: ad_id missing or empty", i),
            );
        }
        if ticket_stage(t).trim().is_empty() {
            no_stage += 1;
            push_sampled(
                &mut samples,
                format!("ticket[{}]: stage missing or empty", i),
            );
        }
        if delivery_epoch(t).is_none() {
            no_delivery += 1;
            push_sampled(
                &mut samples,
                format!(
                    "ticket[{}]: delivery missing or unparseable (epoch seconds or ISO-8601)",
                    i
                ),
            );
        }
    }
    if no_ad + no_stage + no_delivery > 0 {
        r.errors.push(format!(
            "required fields missing: ad_id on {} ticket(s), stage on {}, delivery on {}",
            no_ad, no_stage, no_delivery
        ));
        r.errors.append(&mut samples);
    }
}

fn check_stages(
    tickets: &[&serde_json::Map<String, Value>],
    stages: &[String],
    r: &mut CheckReport,
) {
    // Canonical stages are always recognized; workspace crm.stages extends
    // the set (it may name CRM-specific stages the export passes through).
    let mut known: HashSet<&str> = CANONICAL_STAGES.iter().copied().collect();
    known.extend(stages.iter().map(String::as_str));
    let mut unknown: HashMap<String, usize> = HashMap::new();
    for t in tickets {
        let s = ticket_stage(t);
        if !s.trim().is_empty() && !known.contains(s.as_str()) {
            *unknown.entry(s).or_default() += 1;
        }
    }
    if !unknown.is_empty() {
        let mut names: Vec<_> = unknown.iter().collect();
        names.sort_by(|a, b| b.1.cmp(a.1));
        let list = names
            .iter()
            .take(SAMPLE_CAP)
            .map(|(s, n)| format!("\"{}\" x{}", s, n))
            .collect::<Vec<_>>()
            .join(", ");
        r.warnings.push(format!(
            "unrecognized stage name(s): {} - map to canonical stages ({}) or add to crm.stages in config.json",
            list,
            CANONICAL_STAGES.join(", ")
        ));
    }
}

fn check_delivery_range(tickets: &[&serde_json::Map<String, Value>], r: &mut CheckReport) {
    let epochs: Vec<i64> = tickets.iter().filter_map(|t| delivery_epoch(t)).collect();
    if let (Some(min), Some(max)) = (epochs.iter().min(), epochs.iter().max()) {
        r.info.push(format!(
            "delivery range: {} .. {} ({} ticket(s) with parseable delivery)",
            epoch_date(*min),
            epoch_date(*max),
            epochs.len()
        ));
    }
}

fn epoch_date(ep: i64) -> String {
    Utc.timestamp_opt(ep, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| format!("epoch {}", ep))
}

/// The diagnostic that catches the classic failure: exporting internal or
/// form ids instead of Meta ad ids. 0% join with a snapshot present means
/// brief would show zero CRM leads - that is an error, not a warning.
fn check_join_rate(
    tickets: &[&serde_json::Map<String, Value>],
    snap: &Snapshot,
    r: &mut CheckReport,
) {
    let known: HashSet<&str> = snap.ads_daily.iter().map(|a| a.ad_id.as_str()).collect();
    if known.is_empty() {
        r.info
            .push("snapshot has no ads_daily rows - join-rate check skipped".into());
        return;
    }
    let with_id: Vec<String> = tickets.iter().filter_map(|t| ticket_ad_id(t)).collect();
    let matched = with_id
        .iter()
        .filter(|id| known.contains(id.as_str()))
        .count();
    let pct = (matched as f64 / with_id.len().max(1) as f64) * 100.0;
    let line = format!(
        "ad_id join vs snapshot {}: {}/{} tickets match a known Meta ad ({:.0}%)",
        snap.date,
        matched,
        with_id.len(),
        pct
    );
    if matched == 0 {
        r.errors.push(format!(
            "{} - the export is not using Meta ad ids (form ids and internal ids do not join)",
            line
        ));
    } else if pct < 80.0 {
        r.warnings.push(format!(
            "{} - low join rate; leads outside the snapshot window are normal, but check id source",
            line
        ));
    } else {
        r.info.push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ticket(ad_id: &str, stage: &str, delivery: Value) -> Value {
        serde_json::json!({ "ad_id": ad_id, "stage": stage, "delivery": delivery })
    }

    fn snap_with_ads(ids: &[&str]) -> Snapshot {
        let rows = ids
            .iter()
            .map(|id| serde_json::json!({ "ad_id": id }))
            .collect::<Vec<_>>();
        Snapshot {
            path: PathBuf::new(),
            date: "2026-07-16".into(),
            ads_daily: serde_json::from_value(Value::Array(rows)).unwrap(),
            adsets: Value::Array(vec![]),
            creatives: Value::Array(vec![]),
            crm: Value::Object(Default::default()),
            regions: Value::Array(vec![]),
            changes: Value::Array(vec![]),
            campaigns: Value::Array(vec![]),
        }
    }

    #[test]
    fn valid_flat_array_passes() {
        let crm = Value::Array(vec![
            ticket("111", "Contactable", 1752624000i64.into()),
            ticket(
                "222",
                "Booking",
                Value::String("2026-07-15T09:30:00+05:30".into()),
            ),
        ]);
        let r = check(&crm, &[], None);
        assert!(r.passed(), "errors: {:?}", r.errors);
        assert_eq!(r.tickets, 2);
    }

    #[test]
    fn missing_required_fields_error() {
        let crm = serde_json::json!([{ "stage": "Visit" }]);
        let r = check(&crm, &[], None);
        assert!(!r.passed());
        assert!(r.errors[0].contains("ad_id on 1"));
        assert!(r.errors[0].contains("delivery on 1"));
    }

    #[test]
    fn legacy_map_shape_accepted() {
        let crm = serde_json::json!({
            "camp1": { "tickets": [
                { "adId": { "adId": "111" }, "stage": "Visit", "delivery": 1752624000 }
            ]}
        });
        let r = check(&crm, &[], None);
        assert!(r.passed(), "errors: {:?}", r.errors);
        assert_eq!(r.tickets, 1);
    }

    #[test]
    fn unknown_stage_warns_not_fails() {
        let crm = Value::Array(vec![ticket("111", "Hot Lead", 1752624000i64.into())]);
        let r = check(&crm, &[], None);
        assert!(r.passed());
        assert!(r.warnings.iter().any(|w| w.contains("Hot Lead")));
    }

    #[test]
    fn custom_stage_list_suppresses_warning() {
        let crm = Value::Array(vec![ticket("111", "Hot Lead", 1752624000i64.into())]);
        let stages = vec!["Hot Lead".to_string()];
        let r = check(&crm, &stages, None);
        assert!(r.passed() && r.warnings.is_empty());
    }

    #[test]
    fn non_array_shape_errors() {
        let r = check(&Value::String("nope".into()), &[], None);
        assert!(!r.passed());
        assert!(r.errors[0].contains("array of ticket objects"));
    }

    #[test]
    fn zero_join_rate_is_an_error() {
        let crm = Value::Array(vec![ticket("999", "Visit", 1752624000i64.into())]);
        let snap = snap_with_ads(&["111", "222"]);
        let r = check(&crm, &[], Some(&snap));
        assert!(!r.passed());
        assert!(r.errors.iter().any(|e| e.contains("0/1")));
    }

    #[test]
    fn full_join_rate_passes() {
        let crm = Value::Array(vec![ticket("111", "Visit", 1752624000i64.into())]);
        let snap = snap_with_ads(&["111"]);
        let r = check(&crm, &[], Some(&snap));
        assert!(r.passed(), "errors: {:?}", r.errors);
        assert!(r.info.iter().any(|i| i.contains("100%")));
    }

    #[test]
    fn iso_and_epoch_delivery_both_parse() {
        assert_eq!(
            parse_epoch(&serde_json::json!(1752624000)),
            Some(1752624000)
        );
        assert_eq!(
            parse_epoch(&Value::String("1752624000".into())),
            Some(1752624000)
        );
        assert!(parse_epoch(&Value::String("2026-07-15T09:30:00+05:30".into())).is_some());
        assert!(parse_epoch(&Value::String("not a date".into())).is_none());
    }
}
