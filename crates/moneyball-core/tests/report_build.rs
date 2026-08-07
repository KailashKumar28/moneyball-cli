//! Hermetic build test for the creative report (slice B1): a synthetic
//! snapshot with known ads/creatives/CRM must produce exactly the
//! grouping, funnel, unattributed, and ordering the contract promises.

use moneyball_core::report;
use moneyball_core::schema::*;
use moneyball_core::snapshot::{AdsDailyRow, Snapshot};
use serde_json::json;

fn ad_row(
    ad_id: &str,
    adset: &str,
    name: &str,
    date: &str,
    spend: &str,
    imp: &str,
    leads: u64,
) -> AdsDailyRow {
    serde_json::from_value(json!({
        "campaign_id": "c1", "campaign_name": "NM - Leads",
        "adset_id": format!("as-{}", adset), "adset_name": format!("{} (AI)", adset),
        "ad_id": ad_id, "ad_name": name,
        "spend": spend, "impressions": imp, "inline_link_clicks": "10",
        "reach": "100",
        "actions": [{"action_type": "lead", "value": leads.to_string()}],
        "date_start": date, "date_stop": date,
        "_product": "Namma Mane"
    }))
    .unwrap()
}

fn creative(ad_id: &str, name: &str, hash: Option<&str>, video: bool, active: bool) -> CreativeRow {
    CreativeRow {
        ad_id: ad_id.into(),
        ad_name: name.into(),
        product: "Namma Mane".into(),
        image_hash: hash.map(String::from),
        is_video: video,
        video_id: video.then(|| "v900".into()),
        status: Some(if active { "ACTIVE" } else { "PAUSED" }.into()),
        created_time: Some("2026-08-01T10:00:00+0530".into()),
        asset: hash.map(|h| AssetRef {
            sha256: format!("{:0<64}", h),
            content_type: "image/jpeg".into(),
            bytes: 10,
        }),
        ..Default::default()
    }
}

/// Delivery epoch (seconds) inside the report day for the 2026-08-07
/// snapshot: IST midnight of 08-07 minus half a day = midday 08-06 IST.
fn epoch_in_window() -> i64 {
    // 2026-08-07 00:00 IST = 2026-08-06 18:30 UTC
    let ist_midnight = chrono::NaiveDate::from_ymd_opt(2026, 8, 6)
        .unwrap()
        .and_hms_opt(18, 30, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    ist_midnight - 43200
}

fn snapshot() -> Snapshot {
    // Two image ads sharing one hash (must merge), one video ad with
    // "- Copy 2" (vidname key), one ad with no creative row (ad fallback).
    let ads_daily = vec![
        ad_row(
            "a1",
            "Income",
            "NM Static",
            "2026-08-06",
            "100.5",
            "1000",
            3,
        ),
        ad_row(
            "a2",
            "Broad",
            "NM Static - Copy 2",
            "2026-08-06",
            "50.5",
            "500",
            1,
        ),
        ad_row(
            "a3",
            "Income",
            "NM Walkthrough - Copy 2",
            "2026-08-06",
            "200",
            "2000",
            2,
        ),
        ad_row("a4", "Broad", "Orphan Ad", "2026-08-06", "25", "250", 0),
        // Out-of-window row must be excluded (window=1 covers 08-06 only).
        ad_row("a1", "Income", "NM Static", "2026-08-05", "999", "9999", 9),
    ];
    let ep = epoch_in_window();
    let ticket = |ad_id: &str, stage: &str| json!({"ad_id": ad_id, "stage": stage, "delivery": ep});
    let crm = json!([
        ticket("a1", "Contactable"),           // joins img group, qualified
        ticket("a2", "Registered"),            // joins img group via a2
        ticket("a3", "Booking"),               // video group, full depth
        ticket("zz-not-an-ad", "Contactable"), // unattributed - never dropped
    ]);
    Snapshot {
        path: std::path::PathBuf::from("/t"),
        date: "2026-08-07".into(),
        ads_daily,
        adsets: json!({}),
        creatives: Some(CreativesFile {
            schema: CREATIVES_SCHEMA.into(),
            fetched_at: String::new(),
            rows: vec![
                creative("a1", "NM Static", Some("h1"), false, true),
                creative("a2", "NM Static - Copy 2", Some("h1"), false, false),
                creative("a3", "NM Walkthrough - Copy 2", None, true, true),
            ],
        }),
        crm,
        regions: json!([]),
        changes: json!([]),
        campaigns: json!([]),
    }
}

#[test]
fn build_groups_joins_and_orders_correctly() {
    let r = report::build(&snapshot(), "ws-test", "2026-08-07T00:00:00Z", 1).unwrap();

    assert_eq!(r.schema, CREATIVE_REPORT_SCHEMA);
    assert_eq!(r.window.since, "2026-08-06");
    assert_eq!(r.window.until, "2026-08-06");
    assert_eq!(r.products.len(), 1);
    let p = &r.products[0];

    // 3 groups: shared-hash image pair, video, orphan.
    assert_eq!(
        p.creatives.len(),
        3,
        "{:#?}",
        p.creatives.iter().map(|c| &c.group_key).collect::<Vec<_>>()
    );

    // Ordering: video group has a Booking -> first. Image group has
    // Qualified -> second. Orphan (no CRM) last.
    assert_eq!(p.creatives[0].group_key, "vidname:nm walkthrough");
    assert!(matches!(p.creatives[0].group_kind, GroupKind::VideoName));
    assert_eq!(p.creatives[1].group_key, "img:h1");
    assert_eq!(p.creatives[2].group_key, "ad:a4");

    // Image group merged both ads: spend 151, 2 ads, funnel joined.
    let img = &p.creatives[1];
    assert_eq!(img.ad_ids.len(), 2);
    assert!((img.delivery.spend - 151.0).abs() < 0.01);
    assert_eq!(img.delivery.impressions, 1500); // out-of-window row excluded
    assert_eq!(img.delivery.m_leads, 4);
    let f: Vec<u64> = img.funnel.iter().map(|s| s.count).collect();
    assert_eq!(f, vec![1500, 20, 4, 2, 1, 0, 0]);
    // Status: a1 ACTIVE (no adsets blob) -> Live; image present.
    assert_eq!(img.status.label, "Live");
    assert!(img
        .image
        .as_ref()
        .unwrap()
        .path
        .starts_with("assets/creatives/"));

    // Video group: booking implies visit+qualified per milestones.
    let vid = &p.creatives[0];
    let f: Vec<u64> = vid.funnel.iter().map(|s| s.count).collect();
    assert_eq!(f[3], 1);
    assert_eq!(f[6], 1, "booking counted");

    // Targeting split on the image group: Income (a1) and Broad (a2).
    let t: Vec<&str> = img
        .targetings
        .iter()
        .map(|t| t.targeting.as_str())
        .collect();
    assert_eq!(t, vec!["Broad", "Income"]);

    // Unattributed ticket surfaced at portfolio level.
    assert_eq!(r.portfolio.unattributed_l_leads, 1);
    assert_eq!(r.portfolio.funnel.l_leads, 3);
    assert!((r.portfolio.spend - 376.0).abs() < 0.01);
    assert_eq!(r.portfolio.impressions, 3750);

    // KPI derivations: never zero-as-sentinel.
    assert!(r.portfolio.cost_per_qualified.is_some());
    let orphan = &p.creatives[2];
    assert_eq!(orphan.funnel[3].count, 0);

    // Trend: exactly the window days, funnel counts landed on 08-06.
    assert_eq!(img.trend.len(), 1);
    assert_eq!(img.trend[0].date, "2026-08-06");
    assert_eq!(img.trend[0].l_leads, 2);

    // The whole artifact round-trips through its schema struct.
    let raw = serde_json::to_string(&r).unwrap();
    let back: CreativeReport = serde_json::from_str(&raw).unwrap();
    assert_eq!(back.products[0].creatives.len(), 3);
}

#[test]
fn no_creatives_file_falls_back_to_per_ad_cards() {
    let mut s = snapshot();
    s.creatives = None;
    let r = report::build(&s, "ws", "t", 1).unwrap();
    let p = &r.products[0];
    // Every ad its own card; delivery never dropped.
    assert_eq!(p.creatives.len(), 4);
    assert!(p
        .creatives
        .iter()
        .all(|c| matches!(c.group_kind, GroupKind::AdId)));
    assert!(r.source.creatives_schema.is_none());
}

#[test]
fn empty_crm_flags_source_and_zero_funnel() {
    let mut s = snapshot();
    s.crm = json!({});
    let r = report::build(&s, "ws", "t", 1).unwrap();
    assert!(!r.source.crm_present);
    assert_eq!(r.portfolio.funnel.l_leads, 0);
    assert!(r.portfolio.l_to_q_pct.is_none());
    // Text summary renders without CRM (bot-renderer dry run).
    let txt = report::text_summary(&r);
    assert!(txt.contains("CREATIVE REPORT"), "{}", txt);
}

#[test]
fn html_renders_from_aggregate_and_cache_only() {
    use moneyball_core::report::html;
    // Stage one cached asset so exactly one card gets a data URI.
    let tmp = std::env::temp_dir().join(format!("mb-html-{}", std::process::id()));
    let history = tmp.join("history");
    let assets = history.join("assets").join("creatives");
    let sha = format!("{:0<64}", "h1");
    let dir = assets.join(sha.get(..2).unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{}.jpg", sha)), b"fake-jpeg-bytes").unwrap();

    let r = report::build(&snapshot(), "ws", "2026-08-07T00:00:00Z", 1).unwrap();
    let html = html::render(&r, &history);

    // No unexpanded placeholders, all sections + jumpnav present.
    assert!(!html.contains("{{"), "leftover template placeholder");
    assert!(html.contains(r##"href="#p-namma-mane""##));
    assert!(
        html.contains("data:image/jpeg;base64,"),
        "cached image inlined"
    );
    assert!(
        html.contains("no preview"),
        "missing assets degrade to placeholder"
    );
    assert!(html.contains("VIDEO"), "video tag rendered");
    // Escaping: ad names are data, not markup.
    assert!(!html.contains("<script"));
    std::fs::remove_dir_all(&tmp).ok();
}
