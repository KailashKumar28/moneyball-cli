//! Hermetic: write_snapshot output must parse via snapshot::load with the
//! `_product` tag intact - guarantees the fetcher and reader stay in sync.

use moneyball_core::fetch::write_snapshot;
use moneyball_core::snapshot;

#[test]
fn written_snapshot_round_trips_through_reader() {
    let tmp = std::env::temp_dir().join(format!("mb-fetch-rt-{}", std::process::id()));
    let snap_root = tmp.join("history").join("snap");

    let rows = vec![serde_json::json!({
        "campaign_id": "c1",
        "campaign_name": "Camp One",
        "adset_id": "as1",
        "adset_name": "Set One",
        "ad_id": "a1",
        "ad_name": "Ad One",
        "spend": "123.45",
        "impressions": "1000",
        "clicks": "50",
        "inline_link_clicks": "40",
        "actions": [{"action_type": "lead", "value": "3"}],
        "date_start": "2026-07-15",
        "date_stop": "2026-07-15",
        "_product": "Prod A"
    })];

    let dir = write_snapshot(&snap_root, "2026-07-16", &rows).expect("write ok");
    assert!(dir.join("ads_daily.json").is_file());

    let snap = snapshot::load(&dir).expect("reader parses fetcher output");
    assert_eq!(snap.date, "2026-07-16");
    assert_eq!(snap.ads_daily.len(), 1);
    let r = &snap.ads_daily[0];
    assert_eq!(r._product, "Prod A");
    assert_eq!(r.campaign_id, "c1");
    assert!((r.spend_num() - 123.45).abs() < 1e-9);
    assert_eq!(r.actions.len(), 1);

    // No leftover tmp file from the atomic write.
    assert!(!dir.join("ads_daily.json.tmp").exists());

    let _ = std::fs::remove_dir_all(&tmp);
}
