//! Render the chat-style log via TestBackend to inspect the look.
//! Usage: cargo run --example render_chat -p moneyball-tui

use moneyball_core::brief::{Feasibility, ProductRow};
use moneyball_tui::chat::cells;
use moneyball_tui::chat::{Cell, ChatLog};

fn main() {
    let mut log = ChatLog::new();

    log.push(Cell::System(cells::System(
        "moneyball \u{2022} read-only Meta-ads advisor. Type `/help` for commands, or just ask."
            .into(),
    )));
    log.push(Cell::UserPrompt(cells::UserPrompt {
        text: "/brief".into(),
        at: chrono::Local::now(),
    }));
    log.push(Cell::AssistantText(cells::AssistantText {
        text: "loading portfolio snapshot for 2026-07-13...".into(),
        streaming: false,
    }));
    log.push(Cell::ToolCall(cells::ToolCall {
        name: "brief".into(),
        args: "".into(),
        status: cells::ToolStatus::Done,
    }));

    // Use a Brief cell so the renderer picks single-line vs multi-line
    // based on the chat body width. Mirrors the production /brief path.
    let rows = vec![
        ProductRow {
            product: "Namma Mane".into(),
            spend_per_day: 14067,
            m7d: 166,
            l7d: 154,
            q7d: 35,
            q_per_day: 5.00,
            rs_per_q: Some(2813),
            l_to_q: Some(22.7),
            goal: 10.0,
            gap: 5.0,
            trend: "-".into(),
        },
        ProductRow {
            product: "Valmark CityVille".into(),
            spend_per_day: 4838,
            m7d: 27,
            l7d: 24,
            q7d: 2,
            q_per_day: 0.29,
            rs_per_q: Some(16934),
            l_to_q: Some(8.3),
            goal: 10.0,
            gap: 9.7,
            trend: "-".into(),
        },
        ProductRow {
            product: "Purva Sparkling Spring by Fincity".into(),
            spend_per_day: 15136,
            m7d: 53,
            l7d: 43,
            q7d: 8,
            q_per_day: 1.14,
            rs_per_q: Some(13244),
            l_to_q: Some(18.6),
            goal: 10.0,
            gap: 8.9,
            trend: "-".into(),
        },
        ProductRow {
            product: "Primus by Fincity".into(),
            spend_per_day: 3165,
            m7d: 14,
            l7d: 13,
            q7d: 1,
            q_per_day: 0.14,
            rs_per_q: Some(22152),
            l_to_q: Some(7.7),
            goal: 10.0,
            gap: 9.9,
            trend: "-".into(),
        },
    ];
    let feasibility = Feasibility {
        tot_q_per_day: 6.6,
        tot_spend_per_day: 37206,
        tot_goal_per_day: 40.0,
        cur_rpq: 5662,
        best_rpq: Some(2813),
        required_at_cur: Some(226480),
        required_at_best: Some(112520),
        open_debt: vec!["geo_exclusions_present".into(), "higher_intent_form".into()],
    };
    // We need to create the Brief cell variant. Use a stub BriefView.
    // The /brief handler in lib.rs pushes ToolResult with format_brief_as_lines
    // (multi-line), so for the test we mimic that directly by pushing the
    // multi-line strings as a ToolResult.
    let _rows = rows;
    let _feasibility = feasibility;
    log.push(Cell::ToolResult(cells::ToolResult {
        name: "brief".into(),
        output: vec![
            "BRIEF  (7d window)".into(),
            "".into(),
            "  > Namma Mane".into(),
            "    14,067/d  m 166  l 154  q 35  5.00/d  Rs.2,813".into(),
            "    L\u{2192}Q 22.7%   gap   5.0".into(),
            "  > Valmark CityVille".into(),
            "    4,838/d  m  27  l  24  q  2   0.29/d  Rs.16,934".into(),
            "    L\u{2192}Q  8.3%   gap   9.7".into(),
            "  > Purva Sparkling Spring by Fincity".into(),
            "    15,136/d  m  53  l  43  q  8  1.14/d  Rs.13,244".into(),
            "    L\u{2192}Q 18.6%   gap   8.9".into(),
            "  > Primus by Fincity".into(),
            "    3,165/d  m  14  l  13  q  1   0.14/d  Rs.22,152".into(),
            "    L\u{2192}Q  7.7%   gap   9.9".into(),
            "".into(),
            "FEASIBILITY  6.6 q/day @ Rs.37,206/day = Rs.5,662/q  \u{00B7}  goal 40/day".into(),
            "  required @ current:  Rs.226,480/day (6.1x)".into(),
            "  required @ best Rs.2,813/q: Rs.112,520/day (3.0x)".into(),
            "  setup debt: 2 (geo_exclusions_present, higher_intent_form)".into(),
            "  (87ms)".into(),
        ],
        success: true,
        duration_ms: 87,
    }));

    log.push(Cell::AssistantText(cells::AssistantText {
        text: "Portfolio is at 6.6 q/day against a 40/day goal. At current efficiency you'd need \u{20B9}226k/day; at the best-observed Rs.2,813/q you still need \u{20B9}112k/day. The constraint is money, not channels - want me to propose a goal ramp?".into(),
        streaming: false,
    }));

    log.push(Cell::UserPrompt(cells::UserPrompt {
        text: "yes".into(),
        at: chrono::Local::now(),
    }));

    let lines = log.render(80, 30);
    for ln in lines {
        println!("{}", flatten(&ln));
    }
}

fn flatten(ln: &ratatui::text::Line<'_>) -> String {
    ln.spans.iter().map(|s| s.content.as_ref()).collect()
}
