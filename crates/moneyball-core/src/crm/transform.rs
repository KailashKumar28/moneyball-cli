//! Record -> ticket transform + organic-contact extraction. Split
//! from source.rs (size cap); pure over parsed specs and raw records.

use serde_json::Value;

use super::source::{get_path, scalar_string, MapSpec};

/// Transform raw CRM records into contract tickets. Records with NO ad
/// id are dropped and counted (second return): every real CRM holds
/// organic/direct leads, the contract requires ad_id, and the
/// production LeadZump pipeline keys its export by ad id so such leads
/// never reach moneyball there either (mb.py parity). Placeholder ids
/// like "Stattic Ad" are non-empty and kept untouched (AGENTS.md join
/// rule). Other missing fields become missing keys - the validator
/// reports them per-row afterwards.
pub fn transform(records: &[Value], map: &MapSpec) -> (Vec<Value>, usize) {
    let mut dropped = 0usize;
    let tickets = records
        .iter()
        .filter_map(|rec| {
            let ad_id = get_path(rec, &map.ad_id)
                .map(scalar_string)
                .unwrap_or_default();
            if ad_id.trim().is_empty() {
                dropped += 1;
                return None;
            }
            let mut t = serde_json::Map::new();
            t.insert("ad_id".into(), Value::String(ad_id));
            if let Some(v) = get_path(rec, &map.stage).map(scalar_string) {
                let stage = map.stage_map.get(&v).cloned().unwrap_or(v);
                t.insert("stage".into(), Value::String(stage));
            }
            if let Some(v) = get_path(rec, &map.delivery) {
                t.insert("delivery".into(), v.clone());
            }
            if !map.funnel.is_empty() {
                if let Some(v) = get_path(rec, &map.funnel) {
                    t.insert("funnel".into(), Value::String(scalar_string(v)));
                }
            }
            for (key, path) in [
                ("lead_id", &map.lead_id),
                ("phone", &map.phone),
                ("email", &map.email),
            ] {
                if !path.is_empty() {
                    if let Some(v) = get_path(rec, path) {
                        let sv = scalar_string(v);
                        if !sv.is_empty() {
                            t.insert(key.into(), Value::String(sv));
                        }
                    }
                }
            }
            Some(Value::Object(t))
        })
        .collect();
    (tickets, dropped)
}

/// Contacts of the records `transform` DROPS (no ad id - organic /
/// direct leads). The re-inquiry check needs them: a Meta lead whose
/// phone matches an organic ticket is a returning contact, not a new
/// one. Raw PII per the 2026-08-10 decision; written 0600.
pub fn organic_contacts(records: &[Value], map: &MapSpec) -> Vec<crate::schema::ContactRow> {
    records
        .iter()
        .filter(|rec| {
            get_path(rec, &map.ad_id)
                .map(scalar_string)
                .unwrap_or_default()
                .trim()
                .is_empty()
        })
        .filter_map(|rec| {
            let pick = |path: &str| {
                if path.is_empty() {
                    return None;
                }
                get_path(rec, path)
                    .map(scalar_string)
                    .filter(|s| !s.is_empty())
            };
            let phone = pick(&map.phone);
            let email = pick(&map.email);
            (phone.is_some() || email.is_some())
                .then_some(crate::schema::ContactRow { phone, email })
        })
        .collect()
}
