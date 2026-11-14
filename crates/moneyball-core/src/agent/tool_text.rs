//! What the brief, funnel and diagnose tools return to the model. A
//! typed command hands the model the same text, so both paths read the
//! same numbers. `say` words the fixes in them the front-end's way
//! (`FrontEnd::say`).

use crate::brief::{self, ProductRow, ProductRowsAndFeasibility};
use crate::config::AppConfig;
use crate::crm::status::CrmHealth;
use crate::funnel::{self, By};
use crate::text::commas;
use crate::Snapshot;

/// Where a tool's output turns to the model: the CRM state it must not
/// read past, then an instruction meant for it alone.
const TO_MODEL: &str = "\nIMPORTANT: ";
const MODEL_ONLY: &str = " Do not diagnose lead quality from l/q/v.";

/// What the brief tool returns: the brief's lines, and the CRM state
/// the model must not read past.
pub fn brief_text(b: &ProductRowsAndFeasibility, crm_note: Option<&str>) -> String {
    let mut out = format_brief_as_lines(b).join("\n");
    if let Some(note) = crm_note {
        out.push_str(&format!("{}{}.{}", TO_MODEL, note, MODEL_ONLY));
    }
    out
}

/// A tool's output as a person sees it: the CRM state stays, as a plain
/// warning line; the words addressed to the model do not.
pub fn for_people(output: &str) -> String {
    match output.split_once(TO_MODEL) {
        Some((body, note)) => format!("{}\n! {}", body, note.trim_end_matches(MODEL_ONLY)),
        None => output.to_string(),
    }
}

/// The brief tool's output for a snapshot.
pub(super) fn brief_output(
    cfg: &AppConfig,
    snap: &Snapshot,
    say: &dyn Fn(&str) -> String,
) -> String {
    let history = brief::load_history(&cfg.history_dir().join("scoreboard.csv"));
    let b = brief::compute(snap, cfg, &history);
    let note = CrmHealth::of(cfg, snap).note_for(say);
    brief_text(&b, note.as_deref())
}

/// The tools' answer when there is no snapshot to read. `self_heal`:
/// `FrontEnd::self_heal`, named beside the refresh only where it exists.
pub fn no_snapshot(say: &dyn Fn(&str) -> String, self_heal: Option<&str>) -> String {
    let heals = self_heal.map(|step| format!(" (or {}, which self-heals)", say(step)));
    format!(
        "no snapshot on disk - tell the user to run {}{} before asking for numbers",
        say("/refresh"),
        heals.unwrap_or_default()
    )
}

/// The brief as plain lines: what the model reads and what a front-end
/// shows as the brief tool's result.
pub fn format_brief_as_lines(b: &ProductRowsAndFeasibility) -> Vec<String> {
    let mut out = Vec::new();
    // Header - always dated; stale numbers presented as current mislead.
    out.push(format!("BRIEF  snapshot {}  (7d window)", b.snapshot_date));
    if let Some(warn) = brief::staleness_warning(&b.snapshot_date) {
        out.push(warn);
    }
    out.push(String::new());
    for r in &b.rows {
        product_lines(&mut out, r, b.has_crm);
    }
    if let Some(order) = brief::cpl_order(&b.rows) {
        out.push(format!("  {}", order));
    }
    out.push(String::new());
    feasibility_lines(&mut out, b);
    out
}

/// Per-product block - multi-line so it fits any chat width.
fn product_lines(out: &mut Vec<String>, r: &ProductRow, has_crm: bool) {
    let l_to_q = r
        .l_to_q
        .map(|x| format!("{:.1}%", x))
        .unwrap_or_else(|| "-".into());
    let rs_per_q = r
        .rs_per_q
        .map(|x| format!("Rs.{}/q", commas(x)))
        .unwrap_or_else(|| "-".into());
    out.push(format!("  > {}", r.product));
    if !has_crm {
        // Unknown, not zero: no CRM data means no leads or qualified.
        out.push(format!(
            "    spend Rs.{}/d  meta {}  cpl {}  crm -  qual -",
            commas(r.spend_per_day),
            r.m7d,
            cpl(r)
        ));
        return;
    }
    // Labeled, not single-letter columns: this text is read raw in
    // the chat cell (user feedback: "this is not so readable").
    out.push(format!(
        "    spend Rs.{}/d  meta {}  cpl {}  crm {}  qual {} ({:.2}/d)  {}",
        commas(r.spend_per_day),
        r.m7d,
        cpl(r),
        r.l7d,
        r.q7d,
        r.q_per_day,
        rs_per_q
    ));
    // No goal, no gap: said in words so the model never assumes one (D7).
    let (gap, no_goal) = match r.gap {
        Some(g) => (format!("{:.1}", g), ""),
        None => ("-".into(), "  no goal set"),
    };
    out.push(format!(
        "    L->Q {:>5}   gap {:>5}{}",
        l_to_q, gap, no_goal
    ));
}

fn feasibility_lines(out: &mut Vec<String>, b: &ProductRowsAndFeasibility) {
    let f = &b.feasibility;
    if !b.has_crm {
        out.push("FEASIBILITY  needs qualified leads, and this snapshot has no CRM data".into());
        return;
    }
    let Some(goal) = f.tot_goal_per_day else {
        out.push(format!("FEASIBILITY  {}", brief::print::NEEDS_A_GOAL));
        return;
    };
    out.push(format!(
        "FEASIBILITY  {:.1} q/day @ Rs.{}/day = Rs.{}/q  -  goal {:.0}/day",
        f.tot_q_per_day,
        commas(f.tot_spend_per_day),
        commas(f.cur_rpq),
        goal
    ));
    if let Some(note) = brief::no_goal_note(&b.rows) {
        out.push(format!("  {}", note));
    }
    if let Some(req) = f.required_at_cur {
        out.push(format!(
            "  required @ current:  Rs.{}/day ({:.1}x)",
            commas(req),
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        out.push(format!(
            "  required @ best Rs.{}/q: Rs.{}/day ({:.1}x)",
            commas(b),
            commas(req),
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    let suffix = if f.open_debt.is_empty() {
        String::new()
    } else {
        format!(" ({})", f.open_debt.join(", "))
    };
    out.push(format!("  setup debt: {}{}", f.open_debt.len(), suffix));
}

/// "Rs.1,505", or "-" with no Meta lead to divide by.
fn cpl(r: &ProductRow) -> String {
    r.cpl.map_or("-".into(), |c| format!("Rs.{}", commas(c)))
}

/// What the funnel tool returns: the full table the model reasons from
/// (ids, kill multiple, maturity) and the pre-computed CPL order.
pub fn funnel_output(
    cfg: &AppConfig,
    snap: &Snapshot,
    product: &str,
    by: By,
    say: &dyn Fn(&str) -> String,
) -> String {
    let rows = funnel::compute(snap, cfg, product, 7, by);
    let rank = funnel::cpl_ranking_for_model(&rows)
        .map(|r| format!("\n{}", r))
        .unwrap_or_default();
    let crm = CrmHealth::of(cfg, snap)
        .note_for(say)
        .map(|n| format!("\nIMPORTANT: {}", n))
        .unwrap_or_default();
    format!(
        "FUNNEL {} - by {} - 7d - snapshot {}\n{}{}{}",
        product,
        by.name(),
        snap.date,
        funnel::table(&rows),
        rank,
        crm
    )
}

/// What the diagnose tool returns: the four checks, and the CRM state
/// the model must not read past.
pub fn diagnose_output(
    cfg: &AppConfig,
    snap: &Snapshot,
    product: &str,
    say: &dyn Fn(&str) -> String,
) -> String {
    let mut out = crate::diagnose::text_for(&crate::diagnose::compute(snap, cfg, product), say);
    if let Some(note) = CrmHealth::of(cfg, snap).note_for(say) {
        out.push_str(&format!("IMPORTANT: {}\n", note));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(has_crm: bool) -> ProductRowsAndFeasibility {
        ProductRowsAndFeasibility {
            rows: vec![ProductRow {
                product: "Namma Mane".into(),
                spend_per_day: 12000,
                m7d: 56,
                cpl: Some(1500),
                l7d: 40,
                q7d: 7,
                q_per_day: 1.0,
                rs_per_q: Some(12000),
                l_to_q: Some(17.5),
                goal: Some(3.0),
                gap: Some(-2.0),
                trend: String::new(),
            }],
            feasibility: brief::Feasibility {
                tot_q_per_day: 1.0,
                tot_spend_per_day: 12000,
                tot_goal_per_day: Some(3.0),
                cur_rpq: 12000,
                required_at_cur: Some(36000),
                required_at_best: None,
                best_rpq: None,
                open_debt: vec![],
            },
            // Today's date: no staleness line to depend on the clock.
            snapshot_date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            has_crm,
        }
    }

    #[test]
    fn brief_lines_carry_the_labeled_numbers() {
        let lines = format_brief_as_lines(&brief(true));
        assert!(lines[0].starts_with("BRIEF  snapshot "));
        assert!(lines.contains(
            &"    spend Rs.12,000/d  meta 56  cpl Rs.1,500  crm 40  qual 7 (1.00/d)  Rs.12,000/q"
                .to_string()
        ));
        assert!(lines.contains(&"    L->Q 17.5%   gap  -2.0".to_string()));
        assert!(lines.contains(&"  required @ current:  Rs.36,000/day (3.0x)".to_string()));
        assert_eq!(lines.last().map(String::as_str), Some("  setup debt: 0"));
    }

    /// No CRM data is unknown, never zero, and feasibility says why.
    #[test]
    fn brief_without_crm_shows_dashes_and_the_crm_note() {
        let b = brief(false);
        let lines = format_brief_as_lines(&b);
        assert!(lines
            .contains(&"    spend Rs.12,000/d  meta 56  cpl Rs.1,500  crm -  qual -".to_string()));
        assert!(lines.last().is_some_and(|l| l.contains("no CRM data")));
        let text = brief_text(&b, Some("CRM is 3 days behind"));
        assert!(text.ends_with(
            "\nIMPORTANT: CRM is 3 days behind. Do not diagnose lead quality from l/q/v."
        ));
        // A person keeps the CRM state, not the words meant for the model.
        let shown = for_people(&text);
        assert!(shown.ends_with("\n! CRM is 3 days behind."), "{}", shown);
        assert!(
            !shown.contains("IMPORTANT") && !shown.contains("l/q/v"),
            "{}",
            shown
        );
        assert_eq!(for_people("plain"), "plain");
    }
}
