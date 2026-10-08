//! Snapshot fetcher - pulls daily ad insights from the Meta Marketing API
//! and writes them as a snapshot the reader (`snapshot::load`) consumes.
//!
//! Design boundary: the READ/analysis path (brief, funnel, advisor math)
//! never touches the network - it only sees snapshot files on disk. This
//! module is the one deliberate network writer: an explicit, user-invoked
//! fetch (`moneyball fetch` / `/fetch`) that populates the snapshot dir.
//! It reads from Meta only (GET insights + creative metadata); it never
//! mutates anything on the ad account. `creatives.rs` owns the per-ad
//! creative identity pull (slice A1, docs/CLOUD_PLAN.md); `assets.rs`
//! is the pure content-addressed image cache it fills (slice A2).

pub mod adsets;
pub mod assets;
pub mod creatives;
pub mod leads;
pub(crate) mod map;

use std::path::{Path, PathBuf};

use chrono::{Duration, Local};

use crate::config::AppConfig;
use crate::error::{Error, Result};

pub(crate) const META_GRAPH_BASE: &str = "https://graph.facebook.com";

/// Per-ad daily fields we request. Matches `snapshot::AdsDailyRow` so the
/// reader parses every row without loss.
const INSIGHT_FIELDS: &str = "campaign_id,campaign_name,adset_id,adset_name,ad_id,ad_name,\
spend,impressions,reach,frequency,clicks,inline_link_clicks,inline_link_click_ctr,\
cost_per_inline_link_click,cpc,ctr,cpm,actions";

#[derive(Debug)]
pub struct FetchReport {
    pub date: String,
    pub path: PathBuf,
    /// Per configured product: its row count, or why its ad account
    /// returned nothing (the other products still landed).
    pub per_product: Vec<(String, std::result::Result<usize, String>)>,
    /// Creative rows captured into creatives.json, if that step ran.
    pub creatives: usize,
    /// Rows with a cached image / images downloaded this run.
    pub assets: usize,
    pub assets_downloaded: usize,
    /// A creatives failure never fails the snapshot - it surfaces here.
    pub creatives_error: Option<String>,
    /// Per-lead records captured into leads.json (Diff breakdown).
    pub leads: usize,
    pub leads_error: Option<String>,
    /// Adsets captured into adsets.json (learning stage + targeting).
    pub adsets: usize,
    pub adsets_error: Option<String>,
}

/// Pull `days` of per-ad daily insights for every configured product and
/// write `<workspace>/.moneyball/history/snap/<today>/ads_daily.json` +
/// `creatives.json`. The caller resolves the Meta token (keychain in the
/// CLI/TUI; per-tenant store in a future server) - core never reads
/// ambient secrets for it.
pub fn fetch_snapshot(cfg: &AppConfig, token: &str, days: u32) -> Result<FetchReport> {
    let workspace = cfg
        .workspace
        .as_ref()
        .ok_or_else(|| Error::Config("no workspace configured - run /setup first".into()))?;

    let today = Local::now().date_naive();
    let until = today - Duration::days(1); // snap day itself is excluded from windows
    let since = until - Duration::days(days.max(1) as i64 - 1);
    let date = today.format("%Y-%m-%d").to_string();

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Meta(format!("client: {}", e)))?;

    let (since_s, until_s) = (
        since.format("%Y-%m-%d").to_string(),
        until.format("%Y-%m-%d").to_string(),
    );
    let (all_rows, per_product) = collect_products(&workspace.products, |account| {
        fetch_account_rows(&client, token, account, &since_s, &until_s)
    })?;

    let snap_root = cfg.history_dir().join("snap");
    let path = write_snapshot(&snap_root, &date, &all_rows)?;

    // Creative identity + image capture (slices A1/A2). Best-effort by
    // design: the insights snapshot is the load-bearing artifact; a
    // creatives failure becomes a warning, never a lost snapshot.
    let assets_root = cfg.history_dir().join("assets").join("creatives");
    let accounts: std::collections::BTreeMap<String, String> = workspace
        .products
        .iter()
        .map(|p| (p.name.clone(), act_id(&p.ad_account)))
        .collect();
    let (cr, creatives_error) = match creatives::fetch_and_write(
        &client,
        token,
        &all_rows,
        &accounts,
        &path,
        &snap_root,
        &assets_root,
    ) {
        Ok(r) => (r, None),
        Err(e) => (Default::default(), Some(e.to_string())),
    };

    // Per-lead capture (Diff breakdown). Same best-effort contract: a
    // token without leads_retrieval warns, never loses the snapshot.
    let (leads, leads_error) = match leads::fetch_and_write(&client, token, &all_rows, &path) {
        Ok((n, warn)) => (n, warn),
        Err(e) => (0, Some(e.to_string())),
    };

    // Adset facts: learning stage + targeting specs. Best-effort.
    let (adsets_n, adsets_error) = match adsets::fetch_and_write(&client, token, &all_rows, &path) {
        Ok(n) => (n, None),
        Err(e) => (0, Some(e.to_string())),
    };

    Ok(FetchReport {
        date,
        path,
        per_product,
        adsets: adsets_n,
        adsets_error,
        creatives: cr.rows,
        assets: cr.assets,
        assets_downloaded: cr.downloaded,
        creatives_error,
        leads,
        leads_error,
    })
}

type PerProduct = Vec<(String, std::result::Result<usize, String>)>;

/// Every product's rows, tagged with `_product`. One account failing marks
/// that product with its error and the others still land; only when every
/// account fails (an expired token, no network) does the fetch fail.
fn collect_products(
    products: &[crate::config::Product],
    mut fetch_account: impl FnMut(&str) -> Result<Vec<serde_json::Value>>,
) -> Result<(Vec<serde_json::Value>, PerProduct)> {
    let (mut all, mut per, mut first_err) = (Vec::new(), Vec::new(), None);
    for p in products {
        match fetch_account(&p.ad_account) {
            Ok(rows) => {
                per.push((p.name.clone(), Ok(rows.len())));
                for mut r in rows {
                    if let Some(obj) = r.as_object_mut() {
                        obj.insert("_product".into(), serde_json::Value::String(p.name.clone()));
                    }
                    all.push(r);
                }
            }
            Err(e) => {
                per.push((p.name.clone(), Err(e.to_string())));
                first_err.get_or_insert(e);
            }
        }
    }
    match first_err {
        Some(e) if per.iter().all(|(_, r)| r.is_err()) => Err(e),
        _ => Ok((all, per)),
    }
}

/// One account's per-ad daily rows over [since, until], following paging.
fn fetch_account_rows(
    client: &reqwest::blocking::Client,
    token: &str,
    account_id: &str,
    since: &str,
    until: &str,
) -> Result<Vec<serde_json::Value>> {
    let act = act_id(account_id);
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
        .map_err(|e| crate::meta::net_err("network", e))?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().map_err(|e| crate::meta::net_err("json", e))?;
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

/// Graph account node id, `act_` prefix normalized.
pub(crate) fn act_id(account_id: &str) -> String {
    if account_id.starts_with("act_") {
        account_id.to_string()
    } else {
        format!("act_{}", account_id)
    }
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

#[cfg(test)]
mod collect_tests {
    use super::*;
    use crate::config::Product;

    fn product(name: &str, account: &str) -> Product {
        Product {
            name: name.into(),
            ad_account: account.into(),
        }
    }

    #[test]
    fn one_failing_account_never_aborts_the_others() {
        let products = [product("A", "1"), product("Primus", "2"), product("C", "3")];
        let (rows, per) = collect_products(&products, |acct| match acct {
            "2" => Err(Error::Meta("Meta 400: permission".into())),
            _ => Ok(vec![serde_json::json!({ "ad_id": acct })]),
        })
        .unwrap();
        assert_eq!(rows.len(), 2, "A and C landed");
        assert!(rows.iter().all(|r| r["_product"] != "Primus"));
        assert_eq!(per[0].1, Ok(1));
        assert!(per[1].1.as_ref().unwrap_err().contains("permission"));

        // Every account failing is a token or network problem: fail as before.
        let all_fail =
            collect_products(&products, |_| Err(Error::Meta("Meta 400: code 190".into())));
        assert!(all_fail.unwrap_err().to_string().contains("190"));
    }
}
