//! Funnel compute tests: joins, kill math, levels, the no-CRM gate.

use super::*;
use serde_json::json;
use std::path::PathBuf;

fn snap() -> Snapshot {
    let ads = json!([
        { "campaign_id": "c1", "campaign_name": "Camp One", "adset_id": "s1",
          "adset_name": "Set One", "ad_id": "a1", "ad_name": "Ad One",
          "spend": "9000", "date_start": "2026-07-14", "date_stop": "2026-07-14",
          "_product": "P",
          "actions": [{"action_type": "lead", "value": "4"}] },
        { "campaign_id": "c1", "campaign_name": "Camp One", "adset_id": "s2",
          "adset_name": "Set Two", "ad_id": "a2", "ad_name": "Ad Two",
          "spend": "1000", "date_start": "2026-07-14", "date_stop": "2026-07-14",
          "_product": "P",
          "actions": [] },
        { "campaign_id": "c9", "campaign_name": "Other", "adset_id": "s9",
          "adset_name": "S9", "ad_id": "a9", "ad_name": "A9",
          "spend": "500", "date_start": "2026-07-14", "date_stop": "2026-07-14",
          "_product": "OTHER", "actions": [] }
    ]);
    // 2026-07-14 ~noon IST as epoch (inside the 7d window before 07-16).
    let ep = crate::brief::ist_midnight_epoch(
        NaiveDate::parse_from_str("2026-07-14", "%Y-%m-%d").unwrap(),
    ) + 12 * 3600;
    let crm = json!([
        { "ad_id": "a1", "stage": "Contactable", "delivery": ep },
        { "ad_id": "a1", "stage": "Visit", "delivery": ep },
        { "ad_id": "a2", "stage": "Lost", "delivery": ep },
        { "ad_id": "a9", "stage": "Booking", "delivery": ep },
        { "ad_id": "a1", "stage": "Booking", "delivery": 1 }
    ]);
    Snapshot {
        path: PathBuf::new(),
        date: "2026-07-16".into(),
        ads_daily: serde_json::from_value(ads).unwrap(),
        adsets: json!({ "s1": { "learning_stage_info": { "status": "LEARNING" } } }),
        creatives: None,
        leads: None,
        crm_contacts: None,
        crm,
        regions: json!([]),
        changes: None,
        ads_7d: None,
        campaigns: json!([]),
    }
}

fn cfg() -> AppConfig {
    AppConfig::resolve_optional(Some("/nonexistent-mb-funnel"), None)
}

#[test]
fn aggregates_by_adset_with_crm_join() {
    let rows = compute(&snap(), &cfg(), "P", 7, By::Adset);
    assert_eq!(rows.len(), 2);
    // Sorted by spend desc: s1 first.
    assert_eq!(rows[0].id, "s1");
    assert_eq!(rows[0].m, 4);
    assert_eq!((rows[0].l, rows[0].q, rows[0].v), (2, 2, 1));
    assert_eq!(rows[0].learning, "LEARNING");
    // a9's Booking belongs to another product; a1's epoch-1 ticket is
    // outside the window - neither may leak in.
    assert_eq!(rows[1].id, "s2");
    assert_eq!((rows[1].l, rows[1].q), (1, 0));
}

#[test]
fn kill_logic_follows_the_table() {
    let rows = compute(&snap(), &cfg(), "P", 7, By::Adset);
    // s1: spend 9000, q 2, target 2500 -> mult 3.6 < 6.3 -> keep.
    assert_eq!(rows[0].kill, Some(false));
    assert_eq!(rows[0].kill_mult, 3.6);
    // s2: spend 1000, q 0 -> mult 0.4 < 3.0 -> keep.
    assert_eq!(rows[1].kill, Some(false));
    // sufficient: s1 (spend 9000, m 4) yes; s2 (m 0) no.
    assert!(rows[0].sufficient && !rows[1].sufficient);
}

#[test]
fn no_crm_means_no_verdict() {
    // Spend far past the kill line with zero qualified: with CRM
    // data that is a kill; without it qualified is unknown.
    let mut s = snap();
    s.ads_daily[1] = serde_json::from_value(json!({
        "campaign_id": "c1", "campaign_name": "Camp One", "adset_id": "s2",
        "adset_name": "Set Two", "ad_id": "a2", "ad_name": "Ad Two",
        "spend": "90000", "date_start": "2026-07-14", "date_stop": "2026-07-14",
        "_product": "P", "actions": [] }))
    .unwrap();
    let with_crm = compute(&s, &cfg(), "P", 7, By::Adset);
    assert_eq!(with_crm[0].kill, Some(true), "{:?}", with_crm[0]);
    s.crm = json!([]);
    let rows = compute(&s, &cfg(), "P", 7, By::Adset);
    assert!(rows.iter().all(|r| r.kill.is_none()), "{:?}", rows);
    let table = print::table(&rows);
    assert!(table.contains("no CRM"), "{}", table);

    // The view a person reads: CRM columns unknown ("-"), one verdict,
    // no raw flags, and narrow enough for a 100-column terminal.
    let view = print::view(&rows);
    let first = view.lines().nth(2).unwrap_or_default();
    assert!(
        first.ends_with("no CRM") && first.contains(" -  "),
        "{}",
        view
    );
    assert!(view
        .lines()
        .all(|l| l.chars().count() < 100 || l.starts_with("kill withheld")));
    let call = print::view(&with_crm);
    assert!(
        !call.contains("true") && !call.contains("false"),
        "{}",
        call
    );
    assert!(
        with_crm
            .iter()
            .all(|r| ["kill", "keep"].contains(&print::verdict(r))
                || print::verdict(r).starts_with("wait: ")),
        "{}",
        call
    );
    assert!(
        table.contains("kill withheld"),
        "the reason travels with it: {}",
        table
    );
}

#[test]
fn campaign_level_merges_adsets() {
    let rows = compute(&snap(), &cfg(), "P", 7, By::Campaign);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "c1");
    assert_eq!(rows[0].spend, 10000);
    assert_eq!((rows[0].l, rows[0].q), (3, 2));
    assert_eq!(rows[0].learning, "-");
}

#[test]
fn table_renders_header_and_rows() {
    let rows = compute(&snap(), &cfg(), "P", 7, By::Adset);
    let t = table(&rows);
    assert!(t.starts_with("id"));
    assert!(t.contains("Set One"));
    assert!(!table(&[]).contains("id"));
}
