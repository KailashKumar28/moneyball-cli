//! Per-adset facts for the window's adsets: learning stage (feeds the
//! card status labels), optimization goal, and the targeting spec
//! (geo/age/gender - the report's targeting section). Written as
//! `adsets.json` in the PRE-EXISTING map-by-id shape the readers
//! already consume (`snap.adsets.get(<adset_id>)`) - external
//! pipelines write the same shape, so no envelope here.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::error::{Error, Result};

const IDS_PER_BATCH: usize = 50;

const ADSET_FIELDS: &str = "name,effective_status,learning_stage_info,optimization_goal,\
daily_budget,targeting{geo_locations{cities,regions,custom_locations,countries},\
age_min,age_max,genders}";

/// Fetch facts for every distinct adset in `ads_daily_rows` and write
/// `<snap_dir>/adsets.json`. Returns the adset count. Best-effort by
/// contract (caller warns, never loses the snapshot).
pub fn fetch_and_write(
    client: &reqwest::blocking::Client,
    token: &str,
    ads_daily_rows: &[Value],
    snap_dir: &Path,
) -> Result<usize> {
    let mut ids: Vec<&str> = ads_daily_rows
        .iter()
        .filter_map(|r| r.get("adset_id").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .collect();
    ids.sort_unstable();
    ids.dedup();

    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for chunk in ids.chunks(IDS_PER_BATCH) {
        let resp = client
            .get(format!("{}/", super::META_GRAPH_BASE))
            .query(&[
                ("access_token", token),
                ("ids", &chunk.join(",")),
                ("fields", ADSET_FIELDS),
            ])
            .send()
            .map_err(|e| Error::Meta(format!("adsets network: {}", e)))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .map_err(|e| Error::Meta(format!("adsets json: {}", e)))?;
        if !status.is_success() {
            let msg = body
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Meta(format!("Meta {} for adsets: {}", status, msg)));
        }
        if let Some(obj) = body.as_object() {
            for (id, adset) in obj {
                by_id.insert(id.clone(), adset.clone());
            }
        }
    }

    let final_path = snap_dir.join("adsets.json");
    let tmp = snap_dir.join("adsets.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&by_id)?)?;
    std::fs::rename(&tmp, &final_path)?;
    Ok(by_id.len())
}
