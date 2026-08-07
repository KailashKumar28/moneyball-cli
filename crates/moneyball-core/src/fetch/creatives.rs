//! Per-ad creative identity capture (slice A1, docs/CLOUD_PLAN.md).
//!
//! For the ads that actually delivered in the fetched window, pull each
//! ad's creative facts (image hash / video ids / copy / status) in
//! batched Graph `?ids=` lookups and write them as the versioned
//! `creatives.json` envelope next to ads_daily.json. Facts only - the
//! creative GROUPING key is computed at report time.
//!
//! Images are cached content-addressed via `assets.rs` (slice A2): one
//! download per distinct image, refs reused from the previous
//! snapshot's creatives.json, failures leave `asset: null` (the report
//! renders a placeholder - never an error).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::Value;

use super::assets;
use super::map::{row_from_ad, url_basename};
use crate::error::{Error, Result};
use crate::schema::{AssetRef, CreativeRow, CreativesFile, CREATIVES_SCHEMA};

/// Graph `?ids=` lookups accept at most 50 ids per request.
const IDS_PER_BATCH: usize = 50;

/// Ad fields we request; everything read-only. The nested field params
/// `.thumbnail_width(1080)...` make thumbnail_url full-size instead of
/// 64px - top-level query params do NOT reach the nested creative on a
/// `?ids=` batch (video thumbs are the only image videos have).
const AD_FIELDS: &str = "name,adset_id,campaign_id,effective_status,created_time,\
creative.thumbnail_width(1080).thumbnail_height(1080)\
{image_hash,image_url,thumbnail_url,video_id,object_story_spec,asset_feed_spec,\
title,body,call_to_action_type,instagram_permalink_url}";

/// What the creatives pass accomplished, for the fetch report line.
#[derive(Debug, Default)]
pub struct CreativesReport {
    pub rows: usize,
    /// Rows with a cached image (reused + downloaded).
    pub assets: usize,
    /// Images actually downloaded this run (rest reused from cache).
    pub downloaded: usize,
}

/// Fetch creative facts for every distinct ad in `ads_daily_rows`,
/// cache their images content-addressed under `assets_root`, and write
/// `<snap_dir>/creatives.json`.
pub fn fetch_and_write(
    client: &reqwest::blocking::Client,
    token: &str,
    ads_daily_rows: &[Value],
    accounts: &BTreeMap<String, String>,
    snap_dir: &Path,
    snap_root: &Path,
    assets_root: &Path,
) -> Result<CreativesReport> {
    // Distinct ad_id -> product, insertion-stable for deterministic output.
    let mut ads: BTreeMap<String, String> = BTreeMap::new();
    for r in ads_daily_rows {
        if let Some(id) = r.get("ad_id").and_then(|v| v.as_str()) {
            let product = r.get("_product").and_then(|v| v.as_str()).unwrap_or("");
            ads.entry(id.to_string()).or_insert_with(|| product.into());
        }
    }

    let ids: Vec<&str> = ads.keys().map(String::as_str).collect();
    let mut rows: Vec<CreativeRow> = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(IDS_PER_BATCH) {
        let resp = client
            .get(format!("{}/", super::META_GRAPH_BASE))
            .query(&[
                ("access_token", token),
                ("ids", &chunk.join(",")),
                ("fields", AD_FIELDS),
                // Full-size video/dynamic thumbnails instead of 64px
                // (python fetch_meta.py parity).
                ("thumbnail_width", "1080"),
                ("thumbnail_height", "1080"),
            ])
            .send()
            .map_err(|e| Error::Meta(format!("creatives network: {}", e)))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .map_err(|e| Error::Meta(format!("creatives json: {}", e)))?;
        if !status.is_success() {
            let msg = body
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Meta(format!(
                "Meta {} for creatives: {}",
                status, msg
            )));
        }
        let Some(obj) = body.as_object() else {
            return Err(Error::Meta("creatives: response is not an object".into()));
        };
        for id in chunk {
            // An ad can vanish between insights and this call (deleted);
            // skip it rather than failing the batch.
            if let Some(ad) = obj.get(*id) {
                rows.push(row_from_ad(id, ads.get(*id).map_or("", String::as_str), ad));
            }
        }
    }

    // Full-res upgrade: link/dynamic ads carry an image_hash but no
    // usable image_url on the creative - resolve hashes to full-res
    // URLs via act_X/adimages (python parity). Thumbnail stays as the
    // last-resort visible image if resolution fails.
    resolve_full_res(client, token, accounts, &mut rows);

    // Asset pass: one download per distinct image identity; identities
    // already cached by a prior fetch (or an earlier run today) are
    // reused without touching the network.
    let mut refs = assets::prior_assets(snap_root, assets_root);
    let mut downloaded = 0usize;
    let mut failed: HashMap<String, ()> = HashMap::new(); // don't retry per row
    for row in &mut rows {
        let Some(key) = assets::identity_key(row) else {
            continue;
        };
        if let Some(a) = refs.get(&key) {
            row.asset = Some(a.clone());
            continue;
        }
        if failed.contains_key(&key) {
            continue;
        }
        let Some(url) = row.image_url.as_deref() else {
            continue;
        };
        match download_image(client, url, assets_root) {
            Ok(a) => {
                downloaded += 1;
                refs.insert(key, a.clone());
                row.asset = Some(a);
            }
            Err(_) => {
                // Non-fatal by contract: card renders a placeholder.
                failed.insert(key, ());
            }
        }
    }

    let report = CreativesReport {
        rows: rows.len(),
        assets: rows.iter().filter(|r| r.asset.is_some()).count(),
        downloaded,
    };
    let file = CreativesFile {
        schema: CREATIVES_SCHEMA.into(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        rows,
    };
    write_creatives(snap_dir, &file)?;
    Ok(report)
}

/// Resolve image hashes to full-res URLs per account
/// (`act_X/adimages?hashes=[...]`) and upgrade rows that only have a
/// thumbnail. Best-effort: failures leave the thumbnail URL in place.
fn resolve_full_res(
    client: &reqwest::blocking::Client,
    token: &str,
    accounts: &BTreeMap<String, String>,
    rows: &mut [CreativeRow],
) {
    // product -> distinct hashes still needing a full-res URL.
    let mut want: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for r in rows.iter() {
        if let Some(h) = r.image_hash.as_deref() {
            want.entry(r.product.as_str()).or_default().push(h);
        }
    }
    let mut url_of: BTreeMap<String, String> = BTreeMap::new();
    for (product, mut hashes) in want {
        let Some(act) = accounts.get(product) else {
            continue;
        };
        hashes.sort_unstable();
        hashes.dedup();
        for chunk in hashes.chunks(IDS_PER_BATCH) {
            let hashes_json = serde_json::to_string(chunk).unwrap_or_default();
            let Ok(resp) = client
                .get(format!("{}/{}/adimages", super::META_GRAPH_BASE, act))
                .query(&[
                    ("access_token", token),
                    ("hashes", &hashes_json),
                    ("fields", "hash,url"),
                ])
                .send()
            else {
                continue;
            };
            let Ok(body) = resp.json::<Value>() else {
                continue;
            };
            for im in body
                .get("data")
                .and_then(|d| d.as_array())
                .into_iter()
                .flatten()
            {
                if let (Some(h), Some(u)) = (
                    im.get("hash").and_then(|x| x.as_str()),
                    im.get("url").and_then(|x| x.as_str()),
                ) {
                    url_of.insert(h.to_string(), u.to_string());
                }
            }
        }
    }
    for r in rows.iter_mut() {
        let Some(u) = r.image_hash.as_deref().and_then(|h| url_of.get(h)) else {
            continue;
        };
        r.image_url = Some(u.clone());
        r.image_basename = Some(url_basename(u));
    }
}

/// GET one creative image and store it content-addressed.
fn download_image(
    client: &reqwest::blocking::Client,
    url: &str,
    assets_root: &Path,
) -> Result<AssetRef> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| Error::Meta(format!("image network: {}", e)))?;
    if !resp.status().is_success() {
        return Err(Error::Meta(format!("image HTTP {}", resp.status())));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();
    let bytes = resp
        .bytes()
        .map_err(|e| Error::Meta(format!("image body: {}", e)))?;
    if bytes.is_empty() {
        return Err(Error::Meta("image body empty".into()));
    }
    assets::cache_bytes(assets_root, &bytes, &content_type)
}

/// Temp-then-rename write of the envelope, same crash contract as
/// `write_snapshot`.
fn write_creatives(snap_dir: &Path, file: &CreativesFile) -> Result<()> {
    let final_path = snap_dir.join("creatives.json");
    let tmp = snap_dir.join("creatives.json.tmp");
    let body = serde_json::to_string_pretty(file)
        .map_err(|e| Error::Config(format!("serialize creatives: {}", e)))?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &final_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_is_versioned_envelope_and_loadable() {
        let dir = std::env::temp_dir().join(format!("mb-creatives-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = CreativesFile {
            schema: CREATIVES_SCHEMA.into(),
            fetched_at: "2026-08-07T00:00:00Z".into(),
            rows: vec![row_from_ad("a1", "P", &json!({}))],
        };
        write_creatives(&dir, &file).unwrap();
        let raw = std::fs::read_to_string(dir.join("creatives.json")).unwrap();
        let back: CreativesFile = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.schema, CREATIVES_SCHEMA);
        assert_eq!(back.rows.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
