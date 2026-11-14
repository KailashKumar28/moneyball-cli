//! Goals are optional (D7): a product without a stored goal has no goal
//! and no gap, and feasibility counts only the products that have one.

use super::*;
use crate::agent::tool_text::format_brief_as_lines;
use serde_json::json;
use std::path::PathBuf;

/// Products A and B, every day of the window: A spends 1,000 for 2
/// qualified, B 700 for 2 (the cheaper qualified lead).
fn snap() -> Snapshot {
    let mut ads = vec![];
    let mut crm = vec![];
    for d in 22..=28 {
        let day = format!("2026-09-{}", d);
        for (p, spend) in [("A", "1000"), ("B", "700")] {
            ads.push(json!({ "campaign_id": p, "campaign_name": p, "adset_id": p,
                "adset_name": p, "ad_id": p, "ad_name": p, "spend": spend,
                "date_start": day, "date_stop": day, "_product": p,
                "actions": [{"action_type": "lead", "value": "1"}] }));
            let at = ist_midnight_epoch(NaiveDate::parse_from_str(&day, "%Y-%m-%d").unwrap());
            for _ in 0..2 {
                crm.push(json!({"ad_id": p, "stage": "Contactable", "delivery": at + 3600}));
            }
        }
    }
    Snapshot {
        path: PathBuf::new(),
        date: "2026-09-29".into(),
        ads_daily: serde_json::from_value(json!(ads)).unwrap(),
        adsets: json!({}),
        creatives: None,
        crm: json!(crm),
        leads: None,
        crm_contacts: None,
        regions: json!([]),
        changes: None,
        campaigns: json!([]),
        ads_7d: None,
    }
}

fn brief_with(goals: serde_json::Value) -> ProductRowsAndFeasibility {
    let mut cfg = AppConfig::resolve_optional(Some("/nonexistent-mb-brief-goals"), None);
    cfg.workspace = Some(
        serde_json::from_value(json!({
            "products": [{"name": "A", "ad_account": "1"}, {"name": "B", "ad_account": "2"}],
            "goals": goals,
        }))
        .unwrap(),
    );
    compute(&snap(), &cfg, &[])
}

/// The table's goal and gap cells for one product.
fn goal_gap(b: &ProductRowsAndFeasibility, product: &str) -> (String, String) {
    let table = format_brief_table(&b.rows, b.has_crm);
    let line = table.lines().find(|l| l.starts_with(product)).unwrap();
    let cells: Vec<&str> = line.split_whitespace().collect();
    (cells[9].into(), cells[10].into())
}

#[test]
fn stored_goals_keep_the_math() {
    let b = brief_with(json!({"A": 3.0, "B": 2.0}));
    assert_eq!((b.rows[0].goal, b.rows[0].gap), (Some(3.0), Some(1.0)));
    assert_eq!((b.rows[1].goal, b.rows[1].gap), (Some(2.0), Some(0.0)));
    let f = &b.feasibility;
    assert_eq!(f.tot_goal_per_day, Some(5.0));
    assert_eq!((f.tot_q_per_day, f.tot_spend_per_day), (4.0, 1700));
    assert_eq!((f.cur_rpq, f.best_rpq), (425, Some(350)));
    assert_eq!(
        (f.required_at_cur, f.required_at_best),
        (Some(2125), Some(1750))
    );
    assert_eq!(goal_gap(&b, "A"), ("3".into(), "1".into()));
    assert_eq!(no_goal_note(&b.rows), None);
    let lines = format_brief_as_lines(&b);
    assert!(
        lines.contains(&"    L->Q 100.0%   gap   1.0".into()),
        "{:?}",
        lines
    );
    assert!(
        lines.contains(&"FEASIBILITY  4.0 q/day @ Rs.1,700/day = Rs.425/q  -  goal 5/day".into())
    );
}

#[test]
fn a_product_without_a_goal_has_none_and_is_left_out_of_feasibility() {
    let b = brief_with(json!({"A": 3.0}));
    assert_eq!((b.rows[1].goal, b.rows[1].gap), (None, None));
    assert_eq!(goal_gap(&b, "B"), ("-".into(), "-".into()));
    assert_eq!(goal_gap(&b, "A"), ("3".into(), "1".into()));
    // Only A counts: its spend, its qualified, its cost - B's cheaper
    // qualified lead is not the best-observed.
    let f = &b.feasibility;
    assert_eq!(f.tot_goal_per_day, Some(3.0));
    assert_eq!((f.tot_q_per_day, f.tot_spend_per_day), (2.0, 1000));
    assert_eq!((f.cur_rpq, f.best_rpq), (500, Some(500)));
    assert_eq!(
        (f.required_at_cur, f.required_at_best),
        (Some(1500), Some(1500))
    );
    let note = "no goal set for 1 product - left out of these totals";
    assert_eq!(no_goal_note(&b.rows).as_deref(), Some(note));
    let lines = format_brief_as_lines(&b);
    assert!(
        lines.contains(&"    L->Q 100.0%   gap     -  no goal set".into()),
        "{:?}",
        lines
    );
    assert!(lines.contains(&format!("  {}", note)), "{:?}", lines);
    // Across a process boundary a missing goal is null, never 0 or 10.
    let v = serde_json::to_value(&b).unwrap();
    assert!(v["rows"][1]["goal"].is_null() && v["rows"][1]["gap"].is_null());
    assert_eq!(v["feasibility"]["tot_goal_per_day"], 3.0);
    let agent = print::brief_json(&b, None);
    assert!(agent["products"][1]["goal_per_day"].is_null());
}

#[test]
fn no_goal_at_all_means_no_feasibility() {
    let b = brief_with(json!({}));
    let f = &b.feasibility;
    assert_eq!(f.tot_goal_per_day, None);
    assert_eq!((f.required_at_cur, f.required_at_best), (None, None));
    let lines = format_brief_as_lines(&b);
    assert!(lines.contains(&format!("FEASIBILITY  {}", print::NEEDS_A_GOAL)));
    assert_eq!(
        no_goal_note(&b.rows).as_deref(),
        Some("no goal set for 2 products - left out of these totals")
    );
    assert!(serde_json::to_value(&b).unwrap()["feasibility"]["tot_goal_per_day"].is_null());
}
