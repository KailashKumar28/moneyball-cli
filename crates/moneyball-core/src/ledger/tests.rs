//! Ledger tests: what submit refuses, applied from the change log, the
//! settle verdict, expiry and hand marks.

use super::*;
use crate::schema::{BudgetMove, ChangeRow, ChangesFile, Evidence, Prediction, CHANGES_SCHEMA};
use serde_json::json;
use std::path::PathBuf;

const D: fn(&str) -> NaiveDate = |s| day(s).unwrap();

/// Noon UTC that day, as the advisor's clock reads it.
fn noon(s: &str) -> DateTime<chrono::Local> {
    DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", s))
        .unwrap()
        .with_timezone(&chrono::Local)
}

/// Ad set s1 (campaign c1, ad a1) spends 1,000 a day for the 28 days
/// before `date`: 2 Meta leads a day until 15 Oct, 4 a day after.
fn snap(date: &str, changes: Vec<ChangeRow>) -> Snapshot {
    let ads: Vec<_> = (1..=28)
        .map(|n| {
            let d = D(date) - Duration::days(n);
            let leads = if d > D("2026-10-15") { "4" } else { "2" };
            json!({ "campaign_id": "c1", "campaign_name": "Camp", "adset_id": "s1",
                    "adset_name": "Lookalike", "ad_id": "a1", "ad_name": "Ad", "spend": "1000",
                    "date_start": d.to_string(), "date_stop": d.to_string(), "_product": "P",
                    "actions": [{"action_type": "lead", "value": leads}] })
        })
        .collect();
    Snapshot {
        path: PathBuf::new(),
        date: date.into(),
        ads_daily: serde_json::from_value(json!(ads)).unwrap(),
        adsets: json!({}),
        creatives: None,
        crm: json!([]),
        leads: None,
        crm_contacts: None,
        regions: json!([]),
        changes: Some(ChangesFile {
            schema: CHANGES_SCHEMA.into(),
            fetched_at: String::new(),
            rows: changes,
        }),
        ads_7d: None,
        campaigns: json!([]),
    }
}

fn cfg(test: &str) -> AppConfig {
    let dir = std::env::temp_dir().join(format!("mb-ledger-{}-{}", test, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = AppConfig::resolve_optional(Some(dir.to_str().unwrap()), None);
    cfg.workspace = Some(
        serde_json::from_value(json!({"products": [{"name": "P", "ad_account": "1"}]})).unwrap(),
    );
    cfg
}

/// Cut s1's budget 10%; cost per lead (500 over the week) down to 450.
fn rec() -> Rec {
    Rec {
        product: "P".into(),
        level: "adset".into(),
        entity_id: "s1".into(),
        entity_name: String::new(),
        lever: "budget".into(),
        action: "cut the budget from 1,000 to 900 a day".into(),
        evidence: vec![
            Evidence {
                metric: "meta_leads".into(),
                value: 14.0,
            },
            Evidence {
                metric: "cpl".into(),
                value: 505.0,
            },
        ],
        prediction: Prediction {
            metric: "cpl".into(),
            direction: "down".into(),
            target: 450.0,
            verify_by: "2026-10-22".into(),
        },
        budget: Some(BudgetMove {
            from: 1000.0,
            to: 900.0,
            relearn: false,
        }),
    }
}

fn budget_change(time: &str) -> ChangeRow {
    ChangeRow {
        time: time.into(),
        event_type: "update_ad_set_budget".into(),
        event: "Ad set budget updated".into(),
        object_id: "s1".into(),
        ..Default::default()
    }
}

#[test]
fn submit_refuses_what_the_data_does_not_bear_out() {
    let (cfg, s, now) = (
        cfg("submit"),
        snap("2026-10-15", vec![]),
        noon("2026-10-15"),
    );
    let refused = |r: Rec| submit(&cfg, &s, r, now).unwrap_err().join("; ");

    let mut r = rec();
    r.evidence[0].value = 15.0;
    assert!(refused(r).contains("meta_leads=15 does not match the last 7 days (14.0)"));
    let mut r = rec();
    r.entity_id = "s9".into();
    assert!(refused(r).contains("adset s9 did not deliver"));
    let mut r = rec();
    r.prediction.metric = "qualified".into();
    assert!(refused(r).contains("qualified needs CRM data"));
    let mut r = rec();
    r.prediction.target = 600.0;
    assert!(refused(r).contains("target 600 is not below today's 500.0"));
    let mut r = rec();
    r.budget = Some(BudgetMove {
        from: 1000.0,
        to: 1500.0,
        relearn: false,
    });
    assert!(refused(r).contains("over 20% restarts learning"));
    let mut r = rec();
    r.prediction.verify_by = "2026-10-15".into();
    assert!(refused(r).contains("verify_by must be"));

    assert_eq!(submit(&cfg, &s, rec(), now).unwrap(), "mb-20261015-1");
    let e = &load(&cfg)[0];
    assert_eq!(
        (e.rec.entity_name.as_str(), e.state),
        ("Lookalike", RecState::Proposed)
    );
    assert!(refused(rec()).contains("an open budget rec already targets this adset"));
}

#[test]
fn the_change_log_applies_it_and_the_data_settles_it() {
    let cfg = cfg("settle");
    submit(&cfg, &snap("2026-10-15", vec![]), rec(), noon("2026-10-15")).unwrap();

    // A change on another ad set, and one on s1 that morning before the
    // advice, say nothing about it; one on s1 after it applies it.
    let mut other = budget_change("2026-10-15T13:00:00+0000");
    other.object_id = "s2".into();
    let mut log = vec![other, budget_change("2026-10-15T06:00:00+0000")];
    assert!(
        sweep(&cfg, &snap("2026-10-17", log.clone()), D("2026-10-17"))
            .unwrap()
            .is_empty()
    );
    log.push(budget_change("2026-10-15T14:00:00+0000"));
    let ev = sweep(&cfg, &snap("2026-10-17", log), D("2026-10-17")).unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(
        (ev[0].event, ev[0].date.as_str()),
        (RecState::Applied, "2026-10-15")
    );
    assert_eq!(load(&cfg)[0].applied, Some(D("2026-10-15")));

    // Not due until the snapshot holds 22 Oct whole.
    assert!(sweep(&cfg, &snap("2026-10-22", vec![]), D("2026-10-22"))
        .unwrap()
        .is_empty());
    let ev = sweep(&cfg, &snap("2026-10-24", vec![]), D("2026-10-24")).unwrap();
    assert_eq!(ev[0].event, RecState::Confirmed);
    assert_eq!(ev[0].note.as_deref(), Some("cpl 250 over 16 Oct - 22 Oct"));
    let shown = text(&load(&cfg));
    assert!(shown.contains("0 open, 1 closed"), "{}", shown);
    assert!(
        shown.contains("predicted cpl down to 450; measured cpl 250 over 16 Oct - 22 Oct"),
        "{}",
        shown
    );
    assert!(shown.contains("hit rate: budget 1 of 1 right"), "{}", shown);
}

#[test]
fn unapplied_recs_expire_and_hand_marks_follow_the_states() {
    let cfg = cfg("expire");
    let s = snap("2026-10-15", vec![]);
    submit(&cfg, &s, rec(), noon("2026-10-15")).unwrap();
    let mut r = rec();
    r.lever = "creative".into();
    r.budget = None;
    // Cooling-down is about applied recs; this one is merely open.
    assert_eq!(
        submit(&cfg, &s, r, noon("2026-10-15")).unwrap(),
        "mb-20261015-2"
    );

    assert!(mark(
        &cfg,
        "mb-20261015-2",
        RecState::Confirmed,
        None,
        D("2026-10-16")
    )
    .is_err());
    mark(
        &cfg,
        "mb-20261015-2",
        RecState::Ignored,
        None,
        D("2026-10-16"),
    )
    .unwrap();
    let ev = sweep(&cfg, &snap("2026-10-23", vec![]), D("2026-10-23")).unwrap();
    assert_eq!(ev.len(), 1, "the ignored rec stays ignored");
    assert_eq!(
        (ev[0].id.as_str(), ev[0].event),
        ("mb-20261015-1", RecState::Expired)
    );
    let e = mark(
        &cfg,
        "mb-20261015-1",
        RecState::Applied,
        None,
        D("2026-10-23"),
    );
    assert_eq!(
        e.unwrap_err(),
        "mb-20261015-1 is expired; it cannot become applied"
    );
}

#[test]
fn an_applied_rec_cools_its_entity_down() {
    let cfg = cfg("cooldown");
    submit(&cfg, &snap("2026-10-15", vec![]), rec(), noon("2026-10-15")).unwrap();
    mark(
        &cfg,
        "mb-20261015-1",
        RecState::Applied,
        None,
        D("2026-10-15"),
    )
    .unwrap();
    let mut r = rec();
    r.lever = "targeting".into();
    r.budget = None;
    let why = submit(&cfg, &snap("2026-10-16", vec![]), r, noon("2026-10-16")).unwrap_err();
    assert!(why
        .join("; ")
        .contains("cooling down after mb-20261015-1 until 2026-10-17"));
}
