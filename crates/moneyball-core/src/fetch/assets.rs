//! Content-addressed creative image cache (slice A2, docs/CLOUD_PLAN.md).
//!
//! Layout: `<history>/assets/creatives/<hh>/<sha256>.<ext>` where `hh`
//! is the first two hex chars (directory fan-out). Content addressing
//! makes dedupe free - ten ads sharing one image store one file - and
//! maps 1:1 onto a future object-store key. Files are written
//! temp-then-rename and NEVER deleted during fetch (a later `gc` may
//! drop hashes no snapshot references).
//!
//! This module is pure cache logic - no network. Downloads live in
//! creatives.rs, inside the sanctioned fetch network module.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::schema::{AssetRef, CreativesFile};

/// File extension for a content type; unknown types default to jpg
/// (Meta serves creatives as jpeg unless told otherwise).
pub fn ext_for(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or("").trim() {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    }
}

/// Cache path relative to the assets root: `<hh>/<sha256>.<ext>`.
pub fn rel_path(sha256: &str, content_type: &str) -> PathBuf {
    let hh = sha256.get(..2).unwrap_or("00");
    PathBuf::from(hh).join(format!("{}.{}", sha256, ext_for(content_type)))
}

/// Store bytes content-addressed under `assets_root`; returns the ref.
/// Idempotent: same bytes = same path, existing file is left alone.
pub fn cache_bytes(
    assets_root: &Path,
    bytes: &[u8],
    content_type: &str,
) -> crate::error::Result<AssetRef> {
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let rel = rel_path(&sha256, content_type);
    let path = assets_root.join(&rel);
    if !path.is_file() {
        let dir = path.parent().expect("rel_path always has a parent");
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(AssetRef {
        sha256,
        content_type: content_type.to_string(),
        bytes: bytes.len() as u64,
    })
}

/// The identity fact used to decide "same image, skip the download":
/// Meta's content hash when present, else the CDN basename.
pub fn identity_key(row: &crate::schema::CreativeRow) -> Option<String> {
    row.image_hash
        .as_ref()
        .map(|h| format!("img:{}", h))
        .or_else(|| row.image_basename.as_ref().map(|b| format!("url:{}", b)))
}

/// Asset refs from the most recent prior snapshot that captured
/// creatives, keyed by identity - re-fetching every morning must not
/// re-download unchanged images. Only refs whose cache file still
/// exists count (a wiped cache heals by re-downloading).
pub fn prior_assets(snap_root: &Path, assets_root: &Path) -> HashMap<String, AssetRef> {
    let mut out = HashMap::new();
    let Ok(dates) = crate::snapshot::list_dates(snap_root) else {
        return out;
    };
    for date in dates.iter().rev() {
        let p = snap_root.join(date).join("creatives.json");
        let Ok(raw) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(file) = serde_json::from_str::<CreativesFile>(&raw) else {
            continue;
        };
        for row in &file.rows {
            let (Some(key), Some(asset)) = (identity_key(row), row.asset.clone()) else {
                continue;
            };
            if assets_root
                .join(rel_path(&asset.sha256, &asset.content_type))
                .is_file()
            {
                out.entry(key).or_insert(asset);
            }
        }
        if !out.is_empty() {
            break; // newest snapshot with usable refs is enough
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::CreativeRow;

    #[test]
    fn cache_is_content_addressed_and_idempotent() {
        let root = std::env::temp_dir().join(format!("mb-assets-{}", std::process::id()));
        let a = cache_bytes(&root, b"same-bytes", "image/jpeg").unwrap();
        let b = cache_bytes(&root, b"same-bytes", "image/jpeg").unwrap();
        assert_eq!(a.sha256, b.sha256);
        assert_eq!(a.bytes, 10);
        let path = root.join(rel_path(&a.sha256, &a.content_type));
        assert!(path.is_file());
        // Fan-out dir = first two hash chars.
        assert!(path.parent().unwrap().ends_with(a.sha256.get(..2).unwrap()));
        // Different bytes, different file.
        let c = cache_bytes(&root, b"other-bytes", "image/png").unwrap();
        assert_ne!(c.sha256, a.sha256);
        assert!(root.join(rel_path(&c.sha256, "image/png")).is_file());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ext_mapping_defaults_to_jpg() {
        assert_eq!(ext_for("image/png"), "png");
        assert_eq!(ext_for("image/jpeg; charset=binary"), "jpg");
        assert_eq!(ext_for("application/octet-stream"), "jpg");
    }

    #[test]
    fn identity_prefers_hash_over_basename() {
        let mut r = CreativeRow {
            ad_id: "a".into(),
            image_hash: Some("h1".into()),
            image_basename: Some("x.jpg".into()),
            ..Default::default()
        };
        assert_eq!(identity_key(&r).as_deref(), Some("img:h1"));
        r.image_hash = None;
        assert_eq!(identity_key(&r).as_deref(), Some("url:x.jpg"));
        r.image_basename = None;
        assert!(identity_key(&r).is_none());
    }

    #[test]
    fn prior_assets_reuses_only_existing_files() {
        let tmp = std::env::temp_dir().join(format!("mb-prior-{}", std::process::id()));
        let snap_root = tmp.join("snap");
        let assets_root = tmp.join("assets").join("creatives");
        // A cached file for hash-identity "img:h1".
        let asset = cache_bytes(&assets_root, b"img-bytes", "image/jpeg").unwrap();
        let mk_row = |hash: &str, asset: Option<AssetRef>| CreativeRow {
            ad_id: format!("ad-{}", hash),
            image_hash: Some(hash.into()),
            asset,
            ..Default::default()
        };
        let file = CreativesFile {
            schema: crate::schema::CREATIVES_SCHEMA.into(),
            fetched_at: String::new(),
            rows: vec![
                mk_row("h1", Some(asset.clone())),
                // Ref whose file is gone - must not be reused.
                mk_row(
                    "h2",
                    Some(AssetRef {
                        sha256: "0".repeat(64),
                        content_type: "image/jpeg".into(),
                        bytes: 1,
                    }),
                ),
            ],
        };
        let dir = snap_root.join("2026-08-06");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("creatives.json"),
            serde_json::to_string(&file).unwrap(),
        )
        .unwrap();

        let map = prior_assets(&snap_root, &assets_root);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("img:h1").unwrap().sha256, asset.sha256);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
