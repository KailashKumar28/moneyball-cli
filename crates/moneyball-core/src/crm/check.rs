//! The crm.json conformance validator (docs/CRM_CONTRACT.md): required
//! fields, known stages, the delivery range, and how many tickets join
//! the snapshot's ads. Split from crm/mod.rs (size cap).

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::*;
use crate::snapshot::Snapshot;

#[derive(Debug, Default, serde::Serialize)]
pub struct CheckReport {
    pub tickets: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub info: Vec<String>,
    /// Distinct stage values not in the canonical/configured set, most
    /// frequent first - drives the connect wizard's stage-map step.
    pub unknown_stages: Vec<String>,
}

impl CheckReport {
    pub fn passed(&self) -> bool {
        self.errors.is_empty()
    }

    /// The report as lines, one per finding: "-" info, "warn", "error".
    pub fn lines(&self) -> Vec<String> {
        let tag = |t: &str, l: &String| format!("  {:<5} {}", t, l);
        self.info
            .iter()
            .map(|l| tag("-", l))
            .chain(self.warnings.iter().map(|l| tag("warn", l)))
            .chain(self.errors.iter().map(|l| tag("error", l)))
            .collect()
    }
}

/// `check` with its inputs resolved from the workspace: stage names
/// from config, join target = the latest loadable snapshot (if any).
/// The one prelude for every validation path (fetch, import, connect
/// dry-run, `crm check`) - it was copy-pasted three times before and
/// had already started drifting.
pub fn check_with_workspace(cfg: &crate::AppConfig, crm: &Value) -> CheckReport {
    let stages = cfg
        .workspace
        .as_ref()
        .map(|w| w.crm.stages.clone())
        .unwrap_or_default();
    let snap = cfg
        .snap_for(None)
        .ok()
        .and_then(|p| crate::snapshot::load(&p).ok());
    check(crm, &stages, snap.as_ref())
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
        let mut names: Vec<_> = unknown.into_iter().collect();
        names.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
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
        r.unknown_stages = names.into_iter().map(|(s, _)| s).collect();
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
