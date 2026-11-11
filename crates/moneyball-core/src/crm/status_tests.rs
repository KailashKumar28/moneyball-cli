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
    assert!(all.contains("snapshot Sat 01 Aug"), "{}", all);
    assert!(all.contains("stale: the newest snapshot"), "{}", all);
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
        changes: None,
        ads_7d: None,
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

fn failed(fix: Option<&str>) -> CrmHealth {
    CrmHealth::NoData {
        last: Some("2026-08-12".into()),
        failed: Some(("401 bad key".into(), fix.map(String::from))),
    }
}

/// `note` word for word as it read before `note_for` existed: the TUI,
/// the shell and the model on the TUI all read these.
#[test]
fn note_keeps_its_wording_in_every_state() {
    const UNKNOWN: &str =
        "CRM leads and qualified are unknown, not zero, and verdicts are withheld";
    let no_data = |last: Option<&str>| CrmHealth::NoData {
        last: last.map(String::from),
        failed: None,
    };
    let cases = [
        (CrmHealth::NoSpec, format!("no CRM connected - {UNKNOWN}. Fix: /connect crm (moneyball connect crm in a shell)")),
        (failed(Some("/connect crm")), format!("no CRM data in this snapshot: its CRM pull failed (401 bad key) - {UNKNOWN}. Fix: /connect crm (moneyball connect crm in a shell), then /refresh")),
        (failed(Some("/setup meta")), format!("no CRM data in this snapshot: its CRM pull failed (401 bad key) - {UNKNOWN}. Fix: /setup meta (open moneyball and run /setup meta in a shell), then /refresh")),
        (failed(Some("moneyball crm secret api_key")), format!("no CRM data in this snapshot: its CRM pull failed (401 bad key) - {UNKNOWN}. Fix: moneyball crm secret api_key in a shell, then /refresh")),
        (failed(None), format!("no CRM data in this snapshot: its CRM pull failed (401 bad key) - {UNKNOWN}. Fix: /refresh (moneyball refresh in a shell) once the CRM answers")),
        (no_data(Some("2026-08-12")), format!("no CRM data in this snapshot (newest CRM pull: snapshot 2026-08-12) - {UNKNOWN}. Fix: /refresh (moneyball refresh in a shell)")),
        (no_data(None), format!("no CRM data pulled yet - {UNKNOWN}. Fix: /refresh (moneyball refresh in a shell)")),
    ];
    for (health, said) in cases {
        assert_eq!(health.note(), Some(said));
    }
    assert_eq!(CrmHealth::Fresh.note(), None);
}

/// A front-end with its own words gets them, and no shell aside.
#[test]
fn note_for_says_each_fix_the_front_ends_way() {
    let say = |fix: &str| match fix {
        "/refresh" => "the Refresh button".to_string(),
        f => format!(
            "{} in Terminal",
            f.strip_prefix('/')
                .map_or(f.into(), |c| format!("moneyball {c}"))
        ),
    };
    let note = failed(Some("/connect crm")).note_for(&say).unwrap();
    assert!(
        note.ends_with("Fix: moneyball connect crm in Terminal, then the Refresh button"),
        "{}",
        note
    );
    let note = failed(Some("moneyball crm secret k"))
        .note_for(&say)
        .unwrap();
    assert!(note.ends_with("Fix: moneyball crm secret k in Terminal, then the Refresh button"));
    let note = CrmHealth::NoSpec.note_for(&say).unwrap();
    assert!(
        note.ends_with("Fix: moneyball connect crm in Terminal"),
        "{}",
        note
    );
}
