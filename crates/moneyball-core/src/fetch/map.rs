//! Graph ad object -> CreativeRow mapping (pure; no network).
//! Split from creatives.rs to keep the network module under the size
//! cap and the mapping hermetically testable.

use serde_json::Value;

use crate::schema::CreativeRow;

#[allow(unused_imports)] // url_basename used by resolve_full_res in creatives.rs
/// Map one Graph ad object (with nested `creative`) to a CreativeRow.
/// Missing pieces degrade to None/default - a partial row still groups
/// via the ad_id fallback at report time.
pub(crate) fn row_from_ad(ad_id: &str, product: &str, ad: &Value) -> CreativeRow {
    let cr = ad.get("creative").cloned().unwrap_or(Value::Null);
    let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);

    // Video identity: top-level video_id, else story-spec video_data.
    let video_id = s(&cr, "video_id").or_else(|| {
        cr.pointer("/object_story_spec/video_data/video_id")
            .and_then(|x| x.as_str())
            .map(String::from)
    });
    // Dynamic-creative video ids (asset_feed_spec), sorted + deduped.
    let mut afs_video_ids: Vec<String> = cr
        .pointer("/asset_feed_spec/videos")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.get("video_id").and_then(|x| x.as_str()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    afs_video_ids.sort();
    afs_video_ids.dedup();

    // Image identity: creative.image_hash is only set for directly
    // hash-created creatives; link ads carry it in the story spec, and
    // dynamic creatives under asset_feed_spec.images.
    let image_hash = s(&cr, "image_hash")
        .or_else(|| {
            cr.pointer("/object_story_spec/link_data/image_hash")
                .and_then(|x| x.as_str())
                .map(String::from)
        })
        .or_else(|| {
            cr.pointer("/asset_feed_spec/images/0/hash")
                .and_then(|x| x.as_str())
                .map(String::from)
        });

    let image_url = s(&cr, "image_url").or_else(|| s(&cr, "thumbnail_url"));
    let image_basename = image_url.as_deref().map(url_basename);
    let is_video = video_id.is_some() || !afs_video_ids.is_empty();

    CreativeRow {
        ad_id: ad_id.to_string(),
        adset_id: s(ad, "adset_id").unwrap_or_default(),
        campaign_id: s(ad, "campaign_id").unwrap_or_default(),
        ad_name: s(ad, "name").unwrap_or_default(),
        product: product.to_string(),
        image_hash,
        video_id,
        afs_video_ids,
        image_basename,
        is_video,
        status: s(ad, "effective_status"),
        created_time: s(ad, "created_time"),
        title: s(&cr, "title"),
        body: s(&cr, "body"),
        cta: s(&cr, "call_to_action_type"),
        permalink: s(&cr, "instagram_permalink_url"),
        image_url,
        asset: None, // slice A2 fills the content-addressed cache
    }
}

/// CDN asset filename: path basename, query stripped (the python
/// pipeline's legacy creative identity fallback).
pub(crate) fn url_basename(url: &str) -> String {
    let no_query = url.split_once('?').map_or(url, |(p, _)| p);
    no_query.rsplit('/').next().unwrap_or(no_query).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn image_ad_maps_hash_basename_and_copy() {
        let ad = json!({
            "name": "NM Static - Copy 2",
            "adset_id": "as1", "campaign_id": "c1",
            "effective_status": "ACTIVE",
            "created_time": "2026-06-14T11:02:00+0530",
            "creative": {
                "image_hash": "abc123",
                "image_url": "https://cdn.example/x/487213991_n.jpg?sig=zzz",
                "title": "3BHK villas", "body": "Visit us",
                "call_to_action_type": "LEARN_MORE"
            }
        });
        let r = row_from_ad("a1", "Namma Mane", &ad);
        assert_eq!(r.image_hash.as_deref(), Some("abc123"));
        assert_eq!(r.image_basename.as_deref(), Some("487213991_n.jpg"));
        assert!(!r.is_video && r.video_id.is_none());
        assert_eq!(r.product, "Namma Mane");
        assert_eq!(r.cta.as_deref(), Some("LEARN_MORE"));
        assert!(r.asset.is_none());
    }

    #[test]
    fn video_identity_from_story_spec_and_afs() {
        let ad = json!({
            "name": "NM Video",
            "creative": {
                "object_story_spec": { "video_data": { "video_id": "v9" } },
                "asset_feed_spec": { "videos": [
                    { "video_id": "v2" }, { "video_id": "v1" }, { "video_id": "v2" }
                ]},
                "thumbnail_url": "https://cdn.example/t/thumb.jpg?x=1"
            }
        });
        let r = row_from_ad("a2", "P", &ad);
        assert_eq!(r.video_id.as_deref(), Some("v9"));
        assert_eq!(r.afs_video_ids, vec!["v1", "v2"]); // sorted, deduped
        assert!(r.is_video);
        assert_eq!(r.image_basename.as_deref(), Some("thumb.jpg"));
    }

    #[test]
    fn image_hash_falls_back_to_story_spec_and_afs() {
        let link = json!({ "creative": { "object_story_spec": { "link_data": {
            "image_hash": "lnk1" } } } });
        assert_eq!(
            row_from_ad("a", "P", &link).image_hash.as_deref(),
            Some("lnk1")
        );
        let afs = json!({ "creative": { "asset_feed_spec": { "images": [
            { "hash": "afs1" } ] } } });
        assert_eq!(
            row_from_ad("a", "P", &afs).image_hash.as_deref(),
            Some("afs1")
        );
    }

    #[test]
    fn bare_ad_without_creative_still_rows() {
        let r = row_from_ad("a3", "P", &json!({}));
        assert_eq!(r.ad_id, "a3");
        assert!(!r.is_video && r.image_hash.is_none() && r.image_url.is_none());
    }
}
