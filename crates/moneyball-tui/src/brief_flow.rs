//! /brief: the deterministic portfolio numbers as a tool cell, then the
//! configured LLM's reading of them. Split from commands.rs (size cap);
//! the agent's `brief` tool prints the same lines.

use crate::commands::{app_state_block, call_agent, typed_tool_turn};
use crate::*;
use moneyball_core::brief;
use moneyball_core::text::commas;

/// System prompt for the brief assistant. Persona + output contract.
pub(crate) const BRIEF_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
The user ran /brief: the brief tool's result holds the 7-day per-product numbers (spend, leads, qualified leads, qualified-per-day, Rs/qualified, L->Q %, gap to goal) plus portfolio feasibility math. If it carries a warning (old data, missing CRM data), open with what it says and its fix, in plain words.\n\
Produce a concise commentary that:\n\
  1. Calls out which products are at/over goal and which are under (largest gap first).\n\
  2. Identifies the best and worst Rs/qualified and what that implies for budget reallocation.\n\
  3. Notes the BEST-OBSERVED Rs/qualified vs current - is the gap from inefficient spend or just volume?\n\
  4. Flags setup-debt items if any (they cost leads).\n\
Before suggesting a cut or a budget move away from a product, call the diagnose tool for it.\n\
Keep it tight - 5-8 sentences, no preamble, no bullets unless the user asks.";

/// /brief: the deterministic numbers as a tool cell (so the user sees the
/// source data), then the configured LLM's interpretation. With no
/// snapshot it self-heals through /refresh, which runs this again.
pub(crate) fn run_brief(app: &mut App) {
    use crate::chat::{cells, Cell};
    let started = std::time::Instant::now();
    app.load_brief();
    if let Some(b) = &app.brief {
        let mut out = format_brief_as_lines(b);
        if let Some(note) = &app.crm_note {
            out.push(format!("warn: {}", note));
        }
        let output = brief_text(b, app.crm_note.as_deref());
        app.chat
            .push_tool("brief", "", out, true, started.elapsed().as_millis() as u64);
        let sys = format!("{}\n\n{}", BRIEF_SYSTEM_PROMPT, app_state_block(app));
        let turn = typed_tool_turn(app, "/brief", "brief", serde_json::json!({}), output);
        call_agent(app, &sys, turn);
    } else if moneyball_core::secrets::load_meta_token().is_some() && app.cfg.has_workspace() {
        // No snapshot but we CAN get one ourselves. Agent populates its
        // own data - no dead ends.
        app.status = None;
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: "no snapshot yet - running /refresh to pull one, then the brief...".into(),
            streaming: false,
        }));
        crate::refresh_flow::run_refresh(app, false, true);
    } else {
        // Can't self-fetch (no Meta token / no workspace): explain
        // both paths. Status cleared so the error shows once.
        app.status = None;
        let snap_dir = app.cfg.snap_dir();
        app.chat.push_tool(
            "brief",
            "",
            vec!["no snapshot data and no Meta token to fetch one with.".into()],
            false,
            started.elapsed().as_millis() as u64,
        );
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: format!(
                "I can pull the data myself once Meta is connected: run /setup to paste a \
token, then /refresh (or just /brief) pulls daily insights automatically. Alternatively, point \
your own pipeline at\n\n    {}/<YYYY-MM-DD>/\n\nand /brief reads whatever it writes.",
                snap_dir.display()
            ),
            streaming: false,
        }));
    }
}

/// What the brief tool returns: the lines the cell shows, and the CRM
/// state the model must not read past.
pub(crate) fn brief_text(b: &brief::ProductRowsAndFeasibility, crm_note: Option<&str>) -> String {
    let mut out = format_brief_as_lines(b).join("\n");
    if let Some(note) = crm_note {
        out.push_str(&format!(
            "\nIMPORTANT: {}. Do not diagnose lead quality from l/q/v.",
            note
        ));
    }
    out
}

/// The brief tool's output for a snapshot.
pub(crate) fn brief_output(
    cfg: &moneyball_core::AppConfig,
    snap: &moneyball_core::Snapshot,
) -> String {
    let history = brief::load_history(&cfg.history_dir().join("scoreboard.csv"));
    let b = brief::compute(snap, cfg, &history);
    let note = moneyball_core::crm::status::CrmHealth::of(cfg, snap).note();
    brief_text(&b, note.as_deref())
}

pub(crate) fn format_brief_as_lines(b: &brief::ProductRowsAndFeasibility) -> Vec<String> {
    let mut out = Vec::new();
    // Header - always dated; stale numbers presented as current mislead.
    out.push(format!("BRIEF  snapshot {}  (7d window)", b.snapshot_date));
    if let Some(warn) = brief::staleness_warning(&b.snapshot_date) {
        out.push(warn);
    }
    out.push(String::new());
    // Per-product block - multi-line so it fits any chat width.
    for r in &b.rows {
        let l_to_q = r
            .l_to_q
            .map(|x| format!("{:.1}%", x))
            .unwrap_or_else(|| "-".into());
        let rs_per_q = r
            .rs_per_q
            .map(|x| format!("Rs.{}/q", commas(x)))
            .unwrap_or_else(|| "-".into());
        out.push(format!("  > {}", r.product));
        if !b.has_crm {
            // Unknown, not zero: no CRM data means no leads or qualified.
            out.push(format!(
                "    spend Rs.{}/d  meta {}  cpl {}  crm -  qual -",
                commas(r.spend_per_day),
                r.m7d,
                cpl(r)
            ));
            continue;
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
        out.push(format!(
            "    L->Q {:>5}   gap {:>5}",
            l_to_q,
            format!("{:.1}", r.gap)
        ));
    }
    if let Some(order) = brief::cpl_order(&b.rows) {
        out.push(format!("  cheapest Meta lead first: {}", order));
    }
    out.push(String::new());
    let f = &b.feasibility;
    if !b.has_crm {
        out.push("FEASIBILITY  needs qualified leads, and this snapshot has no CRM data".into());
        return out;
    }
    out.push(format!(
        "FEASIBILITY  {:.1} q/day @ Rs.{}/day = Rs.{}/q  -  goal {:.0}/day",
        f.tot_q_per_day,
        commas(f.tot_spend_per_day),
        commas(f.cur_rpq),
        f.tot_goal_per_day
    ));
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
    out
}

/// "Rs.1,505", or "-" with no Meta lead to divide by.
fn cpl(r: &brief::ProductRow) -> String {
    r.cpl.map_or("-".into(), |c| format!("Rs.{}", commas(c)))
}
