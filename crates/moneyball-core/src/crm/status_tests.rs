//! crm::status tests. Split from status.rs (size cap).

use super::*;
use crate::config::AppConfig;

fn cfg_at(root: &std::path::Path) -> AppConfig {
    AppConfig {
        data_root: root.to_path_buf(),
        date: None,
        workspace: None,
        agent: true,
    }
}

#[test]
fn no_spec_points_at_connect() {
    let tmp = std::env::temp_dir().join(format!("mb-crmst-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let lines = status_lines(&cfg_at(&tmp)).unwrap();
    assert!(lines[0].contains("none"));
    assert!(lines.last().unwrap().contains("crm connect"), "{:?}", lines);
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn stale_crm_json_is_called_out_with_fetch_next() {
    let tmp = std::env::temp_dir().join(format!("mb-crmst2-{}", std::process::id()));
    let mb = tmp.join(".moneyball");
    let snap = mb.join("history").join("snap");
    std::fs::create_dir_all(snap.join("2026-08-01")).unwrap();
    std::fs::create_dir_all(snap.join("2026-08-08")).unwrap();
    std::fs::write(
        mb.join("crm.toml"),
        "name = \"testcrm\"\n[map]\nroot=\"content\"\nad_id=\"a\"\nstage=\"s\"\ndelivery=\"d\"\n",
    )
    .unwrap();
    // crm.json only in the OLD snapshot.
    std::fs::write(
        snap.join("2026-08-01").join("crm.json"),
        r#"[{"ad_id":"1","stage":"Contactable","delivery":1754000000}]"#,
    )
    .unwrap();
    let lines = status_lines(&cfg_at(&tmp)).unwrap();
    let all = lines.join("\n");
    assert!(all.contains("snapshot 2026-08-01"), "{}", all);
    assert!(all.contains("STALE"), "{}", all);
    assert!(all.contains("crm fetch"), "{}", all);
    std::fs::remove_dir_all(&tmp).ok();
}

/// One sentence per state: a connected-but-unfetched CRM is never told
/// to "connect", and the fix names where the data last came from.
#[test]
fn crm_health_says_fetch_not_connect_when_connected() {
    let tmp = std::env::temp_dir().join(format!("mb-crmhealth-{}", std::process::id()));
    let mb = tmp.join(".moneyball");
    let snap_root = mb.join("history").join("snap");
    std::fs::create_dir_all(snap_root.join("2026-08-12")).unwrap();
    std::fs::write(snap_root.join("2026-08-12").join("crm.json"), "[]").unwrap();
    let cfg = cfg_at(&tmp);
    let empty = crate::snapshot::Snapshot {
        path: snap_root.join("2026-09-28"),
        date: "2026-09-28".into(),
        ads_daily: vec![],
        adsets: serde_json::json!({}),
        creatives: None,
        crm: serde_json::json!({}),
        leads: None,
        crm_contacts: None,
        regions: serde_json::json!([]),
        changes: serde_json::json!([]),
        campaigns: serde_json::json!([]),
    };

    // No crm.toml yet: connect.
    let h = CrmHealth::of(&cfg, &empty);
    assert_eq!(h, CrmHealth::NoSpec);
    assert!(h.note().unwrap().contains("/connect crm"));

    // Connected, this snapshot empty: fetch, naming the newest pull.
    std::fs::write(mb.join("crm.toml"), "name = \"t\"\n").unwrap();
    let h = CrmHealth::of(&cfg, &empty);
    assert_eq!(
        h,
        CrmHealth::NoData {
            last: Some("2026-08-12".into()),
            failed: None,
        }
    );
    let note = h.note().unwrap();
    assert!(
        note.contains("/refresh") && note.contains("2026-08-12"),
        "{}",
        note
    );
    assert!(!note.contains("connect"), "{}", note);

    // CRM data present: nothing to say.
    let mut fresh = empty;
    fresh.crm = serde_json::json!([{"ad_id": "1", "stage": "Contactable", "delivery": 1}]);
    assert_eq!(CrmHealth::of(&cfg, &fresh).note(), None);
    std::fs::remove_dir_all(&tmp).ok();
}
