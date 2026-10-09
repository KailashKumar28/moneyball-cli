//! Diagnose tests: stage calls, fatigue, the trend windows, the change
//! log filter, and the text every surface prints.

use super::*;
use crate::schema::{ChangesFile, CHANGES_SCHEMA};
use serde_json::json;
use std::path::PathBuf;

/// One ad-day; `n` is spend, Meta leads, impressions, link clicks, reach.
fn row(day: &str, ad: &str, campaign: &str, n: [&str; 5]) -> serde_json::Value {
    let [spend, leads, impr, clicks, reach] = n;
    json!({ "campaign_id": campaign, "campaign_name": campaign, "adset_id": "s", "adset_name": "S",
            "ad_id": ad, "ad_name": format!("Ad {}", ad), "spend": spend, "impressions": impr,
            "inline_link_clicks": clicks, "reach": reach, "date_start": day, "date_stop": day,
            "_product": "P", "actions": [{"action_type": "lead", "value": leads}] })
}

fn snap(crm: serde_json::Value) -> Snapshot {
    let mut ads = vec![];
    // Two weeks before: cheap leads. The last 7 days: dear ones.
    for d in 1..=21 {
        ads.push(row(
            &format!("2026-09-{:02}", d),
            "a1",
            "Camp One",
            ["1000", "2", "1000", "20", "900"],
        ));
    }
    for d in 22..=28 {
        ads.push(row(
            &format!("2026-09-{:02}", d),
            "a1",
            "Camp One",
            ["1000", "1", "1000", "5", "300"],
        ));
        ads.push(row(
            &format!("2026-09-{:02}", d),
            "a2",
            "Camp Two",
            ["500", "1", "1000", "20", "900"],
        ));
    }
    Snapshot {
        path: PathBuf::new(),
        date: "2026-09-29".into(),
        ads_daily: serde_json::from_value(json!(ads)).unwrap(),
        adsets: json!({}),
        creatives: None,
        crm,
        leads: None,
        crm_contacts: None,
        regions: json!([]),
        changes: Some(ChangesFile {
            schema: CHANGES_SCHEMA.into(),
            fetched_at: String::new(),
            rows: vec![
                ChangeRow {
                    account: "act_1".into(),
                    time: "2026-09-25T06:00:00+0000".into(),
                    event_type: "update_ad_set_budget".into(),
                    event: "Ad set budget updated".into(),
                    object_type: "ad set".into(),
                    object_name: "S".into(),
                    old: Some(
                        json!({"type": "payment_amount", "currency": "INR", "old_value": 500000}),
                    ),
                    new: Some(
                        json!({"type": "payment_amount", "currency": "INR", "new_value": 400000}),
                    ),
                },
                // Too old, and another account: neither shows.
                ChangeRow {
                    account: "act_1".into(),
                    time: "2026-09-01T06:00:00+0000".into(),
                    ..Default::default()
                },
                ChangeRow {
                    account: "act_2".into(),
                    time: "2026-09-26T06:00:00+0000".into(),
                    ..Default::default()
                },
            ],
        }),
        campaigns: json!([]),
    }
}

fn cfg() -> AppConfig {
    let mut cfg = AppConfig::resolve_optional(Some("/nonexistent-mb-diagnose"), None);
    cfg.workspace = Some(
        serde_json::from_value(json!({"products": [{"name": "P", "ad_account": "1"}]})).unwrap(),
    );
    cfg
}

#[test]
fn the_trend_compares_the_last_week_with_the_days_before() {
    let d = compute(&snap(json!([])), &cfg(), "P");
    assert_eq!((d.recent.days, d.before.days), (7, 21));
    assert_eq!((d.recent.m, d.before.m), (14, 42));
    assert_eq!(d.recent.cpl().map(|c| c.round()), Some(750.0));
    assert_eq!(d.before.cpl().map(|c| c.round()), Some(500.0));
    assert!(
        d.stages.is_none() && d.recent.q.is_none(),
        "no CRM: no stages, no qualified"
    );
    let text = text(&d);
    assert!(
        text.contains("cost per Meta lead is up 1.5x on the days before"),
        "{}",
        text
    );
}

#[test]
fn tired_ads_are_called_and_the_change_log_is_this_accounts_week() {
    let d = compute(&snap(json!([])), &cfg(), "P");
    // a1 drew 2% clicks before and 0.5% now, seen 3.3 times a day.
    assert_eq!(d.ads[0].name, "Ad a1");
    assert_eq!(d.ads[0].ctr_before.map(|c| c.round()), Some(2.0));
    assert_eq!(d.ads[0].call, Fatigue::Fatigued);
    assert_eq!(d.ads[1].call, Fatigue::Ok);
    let changes = d.changes.as_ref().unwrap();
    assert_eq!(changes.len(), 1);
    let text = text(&d);
    assert!(
        text.contains("S: Ad set budget updated (Rs.5,000 -> Rs.4,000)"),
        "{}",
        text
    );
}

#[test]
fn stage_calls_follow_where_leads_stop() {
    let at = |stage: &str, ad: &str| json!({"ad_id": ad, "stage": stage, "delivery": 1790000000});
    let mut crm = vec![];
    crm.extend((0..6).map(|_| at("Lost", "a1")));
    crm.extend((0..4).map(|_| at("Contactable", "a1")));
    crm.extend((0..6).map(|_| at("Contactable", "a2")));
    crm.push(at("Visit", "a2"));
    let d = compute(&snap(json!(crm)), &cfg(), "P");
    let stages = d.stages.unwrap();
    let call = |c: &str| stages.iter().find(|r| r.campaign == c).unwrap().call;
    assert_eq!(call("Camp One"), StageCall::Form);
    assert_eq!(call("Camp Two"), StageCall::Sales);
}
