//! crm tests. Split from crm/mod.rs (size cap).

use super::*;
use std::path::PathBuf;

fn ticket(ad_id: &str, stage: &str, delivery: Value) -> Value {
    serde_json::json!({ "ad_id": ad_id, "stage": stage, "delivery": delivery })
}

fn snap_with_ads(ids: &[&str]) -> Snapshot {
    let rows = ids
        .iter()
        .map(|id| serde_json::json!({ "ad_id": id }))
        .collect::<Vec<_>>();
    Snapshot {
        path: PathBuf::new(),
        date: "2026-07-16".into(),
        ads_daily: serde_json::from_value(Value::Array(rows)).unwrap(),
        adsets: Value::Array(vec![]),
        creatives: None,
        leads: None,
        crm_contacts: None,
        crm: Value::Object(Default::default()),
        regions: Value::Array(vec![]),
        changes: Value::Array(vec![]),
        campaigns: Value::Array(vec![]),
    }
}

#[test]
fn valid_flat_array_passes() {
    let crm = Value::Array(vec![
        ticket("111", "Contactable", 1752624000i64.into()),
        ticket(
            "222",
            "Booking",
            Value::String("2026-07-15T09:30:00+05:30".into()),
        ),
    ]);
    let r = check(&crm, &[], None);
    assert!(r.passed(), "errors: {:?}", r.errors);
    assert_eq!(r.tickets, 2);
}

#[test]
fn missing_required_fields_error() {
    let crm = serde_json::json!([{ "stage": "Visit" }]);
    let r = check(&crm, &[], None);
    assert!(!r.passed());
    assert!(r.errors[0].contains("ad_id on 1"));
    assert!(r.errors[0].contains("delivery on 1"));
}

#[test]
fn legacy_map_shape_accepted() {
    let crm = serde_json::json!({
        "camp1": { "tickets": [
            { "adId": { "adId": "111" }, "stage": "Visit", "delivery": 1752624000 }
        ]}
    });
    let r = check(&crm, &[], None);
    assert!(r.passed(), "errors: {:?}", r.errors);
    assert_eq!(r.tickets, 1);
}

#[test]
fn unknown_stage_warns_not_fails() {
    let crm = Value::Array(vec![ticket("111", "Hot Lead", 1752624000i64.into())]);
    let r = check(&crm, &[], None);
    assert!(r.passed());
    assert!(r.warnings.iter().any(|w| w.contains("Hot Lead")));
}

#[test]
fn custom_stage_list_suppresses_warning() {
    let crm = Value::Array(vec![ticket("111", "Hot Lead", 1752624000i64.into())]);
    let stages = vec!["Hot Lead".to_string()];
    let r = check(&crm, &stages, None);
    assert!(r.passed() && r.warnings.is_empty());
}

#[test]
fn non_array_shape_errors() {
    let r = check(&Value::String("nope".into()), &[], None);
    assert!(!r.passed());
    assert!(r.errors[0].contains("array of ticket objects"));
}

#[test]
fn zero_join_rate_is_an_error() {
    let crm = Value::Array(vec![ticket("999", "Visit", 1752624000i64.into())]);
    let snap = snap_with_ads(&["111", "222"]);
    let r = check(&crm, &[], Some(&snap));
    assert!(!r.passed());
    assert!(r.errors.iter().any(|e| e.contains("0/1")));
}

#[test]
fn full_join_rate_passes() {
    let crm = Value::Array(vec![ticket("111", "Visit", 1752624000i64.into())]);
    let snap = snap_with_ads(&["111"]);
    let r = check(&crm, &[], Some(&snap));
    assert!(r.passed(), "errors: {:?}", r.errors);
    assert!(r.info.iter().any(|i| i.contains("100%")));
}

#[test]
fn iso_and_epoch_delivery_both_parse() {
    assert_eq!(
        parse_epoch(&serde_json::json!(1752624000)),
        Some(1752624000)
    );
    assert_eq!(
        parse_epoch(&Value::String("1752624000".into())),
        Some(1752624000)
    );
    assert!(parse_epoch(&Value::String("2026-07-15T09:30:00+05:30".into())).is_some());
    assert!(parse_epoch(&Value::String("not a date".into())).is_none());
}
