//! Snapshot fetcher - pulls daily ad insights from the Meta Marketing API
//! and writes them as a snapshot the reader (`snapshot::load`) consumes.
//!
//! Design boundary: the READ/analysis path (brief, funnel, advisor math)
//! never touches the network - it only sees snapshot files on disk. This
//! module is the one deliberate network writer: an explicit, user-invoked
//! fetch (`moneyball fetch` / `/fetch`) that populates the snapshot dir.
//! It reads from Meta only (GET insights); it never mutates anything on
//! the ad account.

use std::path::{Path, PathBuf};

use chrono::{Duration, Local};

use crate::config::AppConfig;
use crate::error::{Error, Result};

const META_GRAPH_BASE: &str = "https://graph.facebook.com";

/// Per-ad daily fields we request. Matches `snapshot::AdsDailyRow` so the
/// reader parses every row without loss.
const INSIGHT_FIELDS: &str = "campaign_id,campaign_name,adset_id,adset_name,ad_id,ad_name,\
spend,impressions,reach,frequency,clicks,inline_link_clicks,inline_link_click_ctr,\
cost_per_inline_link_click,cpc,ctr,cpm,actions";

#[derive(Debug)]
pub struct FetchReport {
    pub date: String,
    pub path: PathBuf,
    /// (product name, row count) per configured product.
    pub per_product: Vec<(String, usize)>,
}

/// Pull `days` of per-ad daily insights for every configured product and
/// write `<workspace>/moneyball/history/snap/<today>/ads_daily.json`.
/// The Meta token comes from the OS keychain (stored by the wizard).
pub fn fetch_snapshot(cfg: &AppConfig, days: u32) -> Result<FetchReport> {
    let workspace = cfg
        .workspace
        .as_ref()
        .ok_or_else(|| Error::Config("no workspace configured - run /setup first".into()))?;
    let token = crate::secrets::load_meta_token().ok_or_else(|| {
        Error::Secrets("no Meta token in keychain - run /setup to connect Meta".into())
    })?;

    let today = Local::now().date_naive();
    let until = today - Duration::days(1); // snap day itself is excluded from windows
    let since = until - Duration::days(days.max(1) as i64 - 1);
    let date = today.format("%Y-%m-%d").to_string();

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Meta(format!("client: {}", e)))?;

    let mut all_rows: Vec<serde_json::Value> = Vec::new();
    let mut per_product: Vec<(String, usize)> = Vec::new();
    for p in &workspace.products {
        let rows = fetch_account_rows(
            &client,
            &token,
            &p.ad_account,
            &since.format("%Y-%m-%d").to_string(),
            &until.format("%Y-%m-%d").to_string(),
        )?;
        per_product.push((p.name.clone(), rows.len()));
        for mut r in rows {
            if let Some(obj) = r.as_object_mut() {
                obj.insert("_product".into(), serde_json::Value::String(p.name.clone()));
            }
            all_rows.push(r);
        }
    }

    let snap_root = cfg.history_dir().join("snap");
    let path = write_snapshot(&snap_root, &date, &all_rows)?;
    Ok(FetchReport {
        date,
        path,
        per_product,
    })
}

/// One account's per-ad daily rows over [since, until], following paging.
fn fetch_account_rows(
    client: &reqwest::blocking::Client,
    token: &str,
    account_id: &str,
    since: &str,
    until: &str,
) -> Result<Vec<serde_json::Value>> {
    let act = if account_id.starts_with("act_") {
        account_id.to_string()
    } else {
        format!("act_{}", account_id)
    };
    let time_range = format!("{{\"since\":\"{}\",\"until\":\"{}\"}}", since, until);
    let mut url = format!("{}/{}/insights", META_GRAPH_BASE, act);
    let mut first = true;
    let mut out: Vec<serde_json::Value> = Vec::new();
    loop {
        let resp = if first {
            client
                .get(&url)
                .query(&[
                    ("access_token", token),
                    ("level", "ad"),
                    ("time_increment", "1"),
                    ("time_range", &time_range),
                    ("fields", INSIGHT_FIELDS),
                    ("limit", "500"),
                ])
                .send()
        } else {
            // paging.next is a complete URL (token included) - follow as-is.
            client.get(&url).send()
        }
        .map_err(|e| Error::Meta(format!("network: {}", e)))?;
        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| Error::Meta(format!("json: {}", e)))?;
        if !status.is_success() {
            let msg = body
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Meta(format!("Meta {} for {}: {}", status, act, msg)));
        }
        if let Some(rows) = body.get("data").and_then(|d| d.as_array()) {
            out.extend(rows.iter().cloned());
        }
        match body.pointer("/paging/next").and_then(|n| n.as_str()) {
            Some(next) => {
                url = next.to_string();
                first = false;
            }
            None => break,
        }
    }
    Ok(out)
}

/// Write rows to `<snap_root>/<date>/ads_daily.json`. Writes to a temp file
/// then renames so a crash never leaves a half-written snapshot the reader
/// would try to parse.
pub fn write_snapshot(snap_root: &Path, date: &str, rows: &[serde_json::Value]) -> Result<PathBuf> {
    let dir = snap_root.join(date);
    std::fs::create_dir_all(&dir)?;
    let final_path = dir.join("ads_daily.json");
    let tmp_path = dir.join("ads_daily.json.tmp");
    let body = serde_json::to_string_pretty(rows)
        .map_err(|e| Error::Config(format!("serialize snapshot: {}", e)))?;
    std::fs::write(&tmp_path, body)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(dir)
}
