//! Brief presentation - the fixed-width CLI table + feasibility print.
//! Split from the compute module (size cap): compute returns typed
//! rows; this file owns how they look on stdout.

use super::{staleness_warning, ProductRow, ProductRowsAndFeasibility};
use crate::text::commas;

pub fn print_brief(r: &ProductRowsAndFeasibility) {
    println!(
        "BRIEF  snapshot {}  (7d window; config.json goals)",
        r.snapshot_date
    );
    if let Some(warn) = staleness_warning(&r.snapshot_date) {
        println!("{}", warn);
    }
    print!("{}", format_brief_table(&r.rows, r.has_crm));
    if let Some(order) = cpl_order(&r.rows) {
        println!("cheapest Meta lead first: {}", order);
    }
    let f = &r.feasibility;
    println!();
    if !r.has_crm {
        println!("FEASIBILITY  needs qualified leads, and this snapshot has no CRM data");
        print_debt(&f.open_debt);
        return;
    }
    println!(
        "FEASIBILITY  portfolio {:.1} q/day at Rs.{}/day = Rs.{}/qualified - goal {:.0}/day",
        f.tot_q_per_day,
        commas(f.tot_spend_per_day),
        commas(f.cur_rpq),
        f.tot_goal_per_day
    );
    if let Some(req) = f.required_at_cur {
        println!(
            "  required spend at CURRENT efficiency : Rs.{}/day ({:.1}x today)",
            commas(req),
            req as f64 / f.tot_spend_per_day.max(1) as f64
        );
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        println!(
            "  required spend at BEST-OBSERVED Rs.{}/q : Rs.{}/day ({:.1}x today)",
            commas(b),
            commas(req),
            req as f64 / f.tot_spend_per_day.max(1) as f64
        );
    }
    print_debt(&f.open_debt);
}

fn print_debt(debt: &[String]) {
    let suffix = if debt.is_empty() {
        String::new()
    } else {
        format!(" ({})", debt.join(", "))
    };
    println!("  open setup debt: {}{}", debt.len(), suffix);
}

/// Cell-renderer closure type - used by format_brief_table to keep the
/// column-spec vec a readable single-typed alias instead of inline.
type BriefCell = Box<dyn Fn(&ProductRow) -> String>;
type BriefSpec<'a> = (&'a str, BriefCell);

/// Without CRM data the CRM-derived columns print "-": unknown, not zero.
pub fn format_brief_table(rows: &[ProductRow], has_crm: bool) -> String {
    let crm = move |f: fn(&ProductRow) -> String| -> BriefCell {
        Box::new(move |r| if has_crm { f(r) } else { "-".into() })
    };
    let cols: Vec<BriefSpec<'_>> = vec![
        ("product", Box::new(|r| r.product.clone())),
        ("spend_per_day", Box::new(|r| commas(r.spend_per_day))),
        ("m7d", Box::new(|r| r.m7d.to_string())),
        (
            "cpl",
            Box::new(|r| r.cpl.map(commas).unwrap_or_else(|| "-".into())),
        ),
        ("l7d", crm(|r| r.l7d.to_string())),
        ("q7d", crm(|r| r.q7d.to_string())),
        ("q_per_day", crm(|r| fmt_f64(r.q_per_day, 2))),
        (
            "rs_per_q",
            Box::new(|r| r.rs_per_q.map(commas).unwrap_or_else(|| "-".into())),
        ),
        (
            "l_to_q",
            Box::new(|r| {
                r.l_to_q
                    .map(|x| format!("{:.1}", x))
                    .unwrap_or_else(|| "-".into())
            }),
        ),
        ("goal", Box::new(|r| fmt_f64(r.goal, 0))),
        ("gap", crm(|r| fmt_f64(r.gap, 1))),
        ("q_last7_by_day", Box::new(|r| r.trend.clone())),
    ];
    let headers: Vec<&str> = cols.iter().map(|(h, _)| *h).collect();
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| cols.iter().map(|(_, f)| f(r)).collect())
        .collect();
    crate::text::table(&headers, &cells)
}

fn fmt_f64(x: f64, decimals: usize) -> String {
    let scaled = (x * 10f64.powi(decimals as i32)).round() as i64;
    let sign = if scaled < 0 { "-" } else { "" };
    let abs = scaled.unsigned_abs() as f64 / 10f64.powi(decimals as i32);
    format!("{}{}", sign, abs)
}

/// The products by cost per Meta lead, cheapest first ("A Rs.1,505 <
/// B Rs.2,207"), for the model to narrate rather than compute; None
/// with fewer than two to rank.
pub fn cpl_order(rows: &[ProductRow]) -> Option<String> {
    let mut ranked: Vec<(&str, u64)> = rows
        .iter()
        .filter_map(|r| Some((r.product.as_str(), r.cpl?)))
        .collect();
    if ranked.len() < 2 {
        return None;
    }
    ranked.sort_by_key(|(_, c)| *c);
    let parts: Vec<String> = ranked
        .iter()
        .map(|(p, c)| format!("{} Rs.{}", p, crate::text::commas(*c)))
        .collect();
    Some(parts.join(" < "))
}

/// The brief for a machine (MB_AGENT=1): every number a field, and the
/// CRM-derived ones null - unknown, not zero - when the snapshot has no
/// CRM data. `crm_note` says why and what fixes it.
pub fn brief_json(r: &ProductRowsAndFeasibility, crm_note: Option<String>) -> serde_json::Value {
    use serde_json::json;
    let crm = |v: serde_json::Value| {
        if r.has_crm {
            v
        } else {
            serde_json::Value::Null
        }
    };
    let rows: Vec<serde_json::Value> = r
        .rows
        .iter()
        .map(|p| {
            json!({
                "product": p.product,
                "spend_per_day": p.spend_per_day,
                "meta_leads_7d": p.m7d,
                "cost_per_meta_lead": p.cpl,
                "crm_leads_7d": crm(json!(p.l7d)),
                "qualified_7d": crm(json!(p.q7d)),
                "qualified_per_day": crm(json!(p.q_per_day)),
                "cost_per_qualified": p.rs_per_q,
                "lead_to_qualified_pct": p.l_to_q,
                "goal_per_day": p.goal,
                "gap_per_day": crm(json!(p.gap)),
            })
        })
        .collect();
    let f = &r.feasibility;
    json!({
        "snapshot_date": r.snapshot_date,
        "has_crm": r.has_crm,
        "crm_note": crm_note,
        "products": rows,
        "feasibility": crm(json!({
            "qualified_per_day": f.tot_q_per_day,
            "spend_per_day": f.tot_spend_per_day,
            "goal_per_day": f.tot_goal_per_day,
            "cost_per_qualified": f.cur_rpq,
            "spend_needed_at_current": f.required_at_cur,
            "best_cost_per_qualified": f.best_rpq,
            "spend_needed_at_best": f.required_at_best,
        })),
        "setup_debt": f.open_debt,
    })
}
