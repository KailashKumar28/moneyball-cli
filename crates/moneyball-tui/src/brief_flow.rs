//! /brief: the deterministic portfolio numbers as a tool cell, then the
//! configured LLM's reading of them. Split from commands.rs (size cap);
//! the agent's `brief` tool prints the same lines.

use crate::commands::{app_state_block, call_agent};
use crate::*;
use moneyball_core::brief;

/// System prompt for the brief assistant. Persona + output contract.
pub(crate) const BRIEF_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
You will be given 7-day per-product numbers (spend, leads, qualified leads, qualified-per-day, Rs/qualified, L->Q %, goal, gap) plus portfolio feasibility math.\n\
Produce a concise commentary that:\n\
  1. Calls out which products are at/over goal and which are under (largest gap first).\n\
  2. Identifies the best and worst Rs/qualified and what that implies for budget reallocation.\n\
  3. Notes the BEST-OBSERVED Rs/qualified vs current - is the gap from inefficient spend or just volume?\n\
  4. Flags setup-debt items if any (they cost leads).\n\
Keep it tight - 5-8 sentences, no preamble, no bullets unless the user asks.";

/// /brief: the deterministic numbers as a tool cell (so the user sees the
/// source data), then the configured LLM's interpretation. With no
/// snapshot it self-heals through /refresh, which runs this again.
pub(crate) fn run_brief(app: &mut App) {
    use crate::chat::{cells, Cell};
    let started = std::time::Instant::now();
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: "loading portfolio snapshot...".into(),
        streaming: false,
    }));
    app.load_brief();
    if let Some(b) = &app.brief {
        let mut out = format_brief_as_lines(b);
        let mut user_prompt = build_brief_prompt(b);
        if let Some(note) = &app.crm_note {
            out.push(format!("warn: {}", note));
            user_prompt.push_str(&format!(
                "\n\nIMPORTANT: {}. Do not diagnose lead quality from l/q/v.",
                note
            ));
        }
        app.chat
            .push_tool("brief", "", out, true, started.elapsed().as_millis() as u64);
        let sys = format!("{}\n\n{}", BRIEF_SYSTEM_PROMPT, app_state_block(app));
        call_agent(app, &sys, &user_prompt);
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
        let snap_dir = app.cfg.history_dir().join("snap");
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

/// Build the user-prompt context from the deterministic brief numbers.
/// The LLM sees exactly the numbers that appear in the chat tool cell.
pub(crate) fn build_brief_prompt(b: &brief::ProductRowsAndFeasibility) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Snapshot portfolio dated {} (7d window). Goal = qualified leads per day.\n",
        b.snapshot_date
    ));
    if let Some(warn) = brief::staleness_warning(&b.snapshot_date) {
        s.push_str(&format!(
            "{}\nOpen the commentary by telling the user this.\n",
            warn
        ));
    }
    s.push('\n');
    for r in &b.rows {
        if !b.has_crm {
            s.push_str(&format!(
                "- {}: spend Rs.{}/day, m={}, CRM leads and qualified unknown\n",
                r.product, r.spend_per_day, r.m7d
            ));
            continue;
        }
        let l_to_q = r
            .l_to_q
            .map(|x| format!("{:.1}%", x))
            .unwrap_or_else(|| "-".into());
        let rs_per_q = r
            .rs_per_q
            .map(|x| format!("Rs.{}", x))
            .unwrap_or_else(|| "-".into());
        s.push_str(&format!(
            "- {}: spend Rs.{}/day, m={}, l={}, q={} ({:.2}/day), {}, L->Q {}, goal={}, gap={:.1}\n",
            r.product,
            r.spend_per_day,
            r.m7d,
            r.l7d,
            r.q7d,
            r.q_per_day,
            rs_per_q,
            l_to_q,
            r.goal,
            r.gap,
        ));
    }
    let f = &b.feasibility;
    if !b.has_crm {
        s.push_str(&format!(
            "\nNo CRM data in this snapshot: qualified per day, Rs/q and the gap to the \
             {:.0}/day goal are unknown, not zero.\n\nWrite the commentary.",
            f.tot_goal_per_day
        ));
        return s;
    }
    s.push_str(&format!(
        "\nPortfolio: {:.1} q/day at Rs.{}/day = Rs.{}/q. Goal: {:.0}/day.\n",
        f.tot_q_per_day, f.tot_spend_per_day, f.cur_rpq, f.tot_goal_per_day
    ));
    if let Some(req) = f.required_at_cur {
        s.push_str(&format!(
            "To hit goal at current efficiency: Rs.{}/day ({:.1}x today).\n",
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        s.push_str(&format!(
            "To hit goal at best-observed Rs.{}/q: Rs.{}/day ({:.1}x today).\n",
            b,
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if !f.open_debt.is_empty() {
        s.push_str(&format!("Open setup debt: {}\n", f.open_debt.join(", ")));
    }
    s.push_str("\nWrite the commentary.");
    s
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
            .map(|x| format!("Rs.{}/q", x))
            .unwrap_or_else(|| "-".into());
        out.push(format!("  > {}", r.product));
        if !b.has_crm {
            // Unknown, not zero: no CRM data means no leads or qualified.
            out.push(format!(
                "    spend Rs.{}/d  meta {}  crm -  qual -",
                r.spend_per_day, r.m7d
            ));
            continue;
        }
        // Labeled, not single-letter columns: this text is read raw in
        // the chat cell (user feedback: "this is not so readable").
        out.push(format!(
            "    spend Rs.{}/d  meta {}  crm {}  qual {} ({:.2}/d)  {}",
            r.spend_per_day, r.m7d, r.l7d, r.q7d, r.q_per_day, rs_per_q
        ));
        out.push(format!(
            "    L->Q {:>5}   gap {:>5}",
            l_to_q,
            format!("{:.1}", r.gap)
        ));
    }
    out.push(String::new());
    let f = &b.feasibility;
    if !b.has_crm {
        out.push("FEASIBILITY  needs qualified leads, and this snapshot has no CRM data".into());
        return out;
    }
    out.push(format!(
        "FEASIBILITY  {:.1} q/day @ Rs.{}/day = Rs.{}/q  -  goal {:.0}/day",
        f.tot_q_per_day, f.tot_spend_per_day, f.cur_rpq, f.tot_goal_per_day
    ));
    if let Some(req) = f.required_at_cur {
        out.push(format!(
            "  required @ current:  Rs.{}/day ({:.1}x)",
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        out.push(format!(
            "  required @ best Rs.{}/q: Rs.{}/day ({:.1}x)",
            b,
            req,
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
