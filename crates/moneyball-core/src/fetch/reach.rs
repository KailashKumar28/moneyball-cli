//! Each ad's reach and frequency over the 7 days before the snapshot day,
//! counted by Meta across the week (people deduplicated) - what tells
//! /diagnose how often a person saw an ad, which daily rows cannot.
//! Written as `ads_7d.json` (moneyball.ads_7d/1).

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::error::Result;
use crate::schema::{Ad7dRow, Ads7dFile, ADS_7D_SCHEMA};

/// Pull every account's per-ad week over [since, until] and write
/// `<snap_dir>/ads_7d.json`. Returns the row count and, when some
/// account could not be read, why (the others still land). Err when none
/// could, or the file cannot be written; the caller warns, never loses
/// the snapshot.
pub fn fetch_and_write(
    client: &reqwest::blocking::Client,
    token: &str,
    accounts: &BTreeSet<String>,
    (since, until): (&str, &str),
    snap_dir: &Path,
) -> Result<(usize, Option<String>)> {
    let time_range = format!("{{\"since\":\"{}\",\"until\":\"{}\"}}", since, until);
    let accounts: Vec<&String> = accounts.iter().collect();
    let pulled = super::in_parallel(&accounts, |act| {
        let first = client
            .get(format!("{}/{}/insights", super::META_GRAPH_BASE, act))
            .query(&[
                ("access_token", token),
                ("level", "ad"),
                ("time_range", &time_range),
                ("fields", "ad_id,reach,impressions,frequency"),
                ("limit", "500"),
            ]);
        crate::meta::get_all(client, first, "the 7-day reach", crate::meta::PULL)
    });
    let (rows, failed) = super::per_account(&accounts, pulled, |_, e| row_of(e))?;
    let file = Ads7dFile {
        schema: ADS_7D_SCHEMA.into(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        since: since.into(),
        until: until.into(),
        rows,
    };
    let tmp = snap_dir.join("ads_7d.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&file)?)?;
    std::fs::rename(&tmp, snap_dir.join("ads_7d.json"))?;
    Ok((file.rows.len(), failed))
}

/// Meta sends the numbers as strings.
fn row_of(e: &Value) -> Ad7dRow {
    let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("");
    Ad7dRow {
        ad_id: text("ad_id").to_string(),
        reach: text("reach").parse().unwrap_or(0),
        impressions: text("impressions").parse().unwrap_or(0),
        frequency: text("frequency").parse().unwrap_or(0.0),
    }
}
