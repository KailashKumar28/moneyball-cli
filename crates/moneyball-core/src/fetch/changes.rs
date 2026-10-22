//! Each ad account's change log over the fetched window
//! (`act_X/activities`): what was created, paused, re-budgeted or
//! re-targeted, and when - the "what changed" check of /diagnose reads
//! it. Written as `changes.json` (moneyball.changes/1), newest first.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::error::Result;
use crate::schema::{ChangeRow, ChangesFile, CHANGES_SCHEMA};

const FIELDS: &str =
    "event_type,translated_event_type,event_time,object_id,object_name,object_type,extra_data";

/// Pull every account's log from `since` (epoch seconds; the week
/// /diagnose reads, not the whole fetch window) and write
/// `<snap_dir>/changes.json`. Returns the row count and, when some
/// account's log could not be read, why (the others still land). Err
/// when no account's could, or the file cannot be written; the caller
/// warns, never loses the snapshot.
pub fn fetch_and_write(
    client: &reqwest::blocking::Client,
    token: &str,
    accounts: &BTreeSet<String>,
    since: i64,
    snap_dir: &Path,
) -> Result<(usize, Option<String>)> {
    let since = since.to_string();
    let accounts: Vec<&String> = accounts.iter().collect();
    // Accounts at once: one account's log can take 15s to answer.
    let pulled = super::in_parallel(&accounts, |act| {
        let first = client
            .get(format!("{}/{}/activities", super::META_GRAPH_BASE, act))
            .query(&[
                ("access_token", token),
                ("fields", FIELDS),
                ("since", &since),
                ("limit", "500"),
            ]);
        crate::meta::get_all(client, first, "the change log", crate::meta::PULL)
    });
    let (mut rows, failed) = super::per_account(&accounts, pulled, row_of)?;
    // RFC3339 with one offset sorts as text.
    rows.sort_by(|a, b| b.time.cmp(&a.time));
    let file = ChangesFile {
        schema: CHANGES_SCHEMA.into(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        rows,
    };
    let tmp = snap_dir.join("changes.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&file)?)?;
    std::fs::rename(&tmp, snap_dir.join("changes.json"))?;
    Ok((file.rows.len(), failed))
}

fn row_of(account: &str, e: &Value) -> ChangeRow {
    let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let event_type = text("event_type");
    // extra_data arrives as a JSON string.
    let extra: Value = e
        .get("extra_data")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let value = |k: &str| extra.get(k).filter(|v| !v.is_null()).cloned();
    let object_type = match text("object_type").as_str() {
        "CAMPAIGN_GROUP" => "campaign".to_string(),
        "CAMPAIGN" => "ad set".to_string(),
        "ADGROUP" => "ad".to_string(),
        other => other.to_lowercase().replace('_', " "),
    };
    ChangeRow {
        account: account.to_string(),
        time: text("event_time"),
        event: match text("translated_event_type") {
            t if t.is_empty() => event_type.replace('_', " "),
            t => t,
        },
        event_type,
        object_type,
        object_name: text("object_name"),
        object_id: text("object_id"),
        old: value("old_value"),
        new: value("new_value"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logged_change_reads_in_plain_words() {
        let e = serde_json::json!({
            "event_type": "update_ad_set_budget",
            "translated_event_type": "Ad set budget updated",
            "event_time": "2026-10-08T11:18:00+0000",
            "object_type": "CAMPAIGN",
            "object_name": "Lookalike",
            "extra_data": "{\"old_value\":{\"type\":\"payment_amount\",\"currency\":\"INR\",\"old_value\":500000},\"new_value\":{\"type\":\"payment_amount\",\"currency\":\"INR\",\"new_value\":400000}}"
        });
        let r = row_of("act_1", &e);
        assert_eq!(
            (
                r.object_type.as_str(),
                r.event.as_str(),
                r.object_name.as_str()
            ),
            ("ad set", "Ad set budget updated", "Lookalike")
        );
        assert_eq!(r.old.unwrap()["old_value"], 500000);
    }
}
