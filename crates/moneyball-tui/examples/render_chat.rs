//! Render the chat-style log via TestBackend to inspect the look.
//! Usage: cargo run --example render_chat -p moneyball-tui

use moneyball_tui::chat::cells;
use moneyball_tui::chat::{Cell, ChatLog};

fn main() {
    let mut log = ChatLog::new();

    // Welcome system message
    log.push(Cell::System(cells::System(
        "moneyball \u{2022} read-only Meta-ads advisor. Type `/help` for commands, or just ask."
            .into(),
    )));

    // User asks a question
    log.push(Cell::UserPrompt(cells::UserPrompt {
        text: "/brief".into(),
        at: chrono::Local::now(),
    }));

    // Assistant narration
    log.push(Cell::AssistantText(cells::AssistantText {
        text: "Loading portfolio snapshot for 2026-07-13...".into(),
        streaming: false,
    }));

    // Tool call: brief
    log.push(Cell::ToolCall(cells::ToolCall {
        name: "brief".into(),
        args: "".into(),
        status: cells::ToolStatus::Done,
    }));
    log.push(Cell::ToolResult(cells::ToolResult {
        name: "brief".into(),
        output: vec![
            "Namma Mane              14,067/d  m 166  l 154  q 35   5.00/d  \u{20B9}2,813/q  L\u{2192}Q 22.7%   gap  5.0".into(),
            "Valmark CityVille        4,838/d  m  27  l  24  q  2   0.29/d  \u{20B9}16,934/q  L\u{2192}Q  8.3%   gap  9.7".into(),
            "Purva Sparkling Sprin\u{2026}  15,136/d  m  53  l  43  q  8   1.14/d  \u{20B9}13,244/q  L\u{2192}Q 18.6%   gap  8.9".into(),
            "Primus by Fincity        3,165/d  m  14  l  13  q  1   0.14/d  \u{20B9}22,152/q  L\u{2192}Q  7.7%   gap  9.9".into(),
            "".into(),
            "FEASIBILITY  6.6 q/day @ \u{20B9}37,206/day = \u{20B9}5,662/q \u{00B7} goal 40/day".into(),
            "  required @ current:  \u{20B9}226,480/day (6.1x)".into(),
            "  required @ best \u{20B9}2,813/q: \u{20B9}112,520/day (3.0x)".into(),
            "  setup debt: 2 (geo_exclusions_present, higher_intent_form)".into(),
        ],
        success: true,
        duration_ms: 87,
    }));

    // Assistant summary
    log.push(Cell::AssistantText(cells::AssistantText {
        text:
            "Portfolio is at 6.6 q/day against a 40/day goal. At current efficiency you'd need \u{20B9}226k/day; \
             at the best-observed \u{20B9}2,813/q you still need \u{20B9}112k/day. The constraint is money, not channels \
             - want me to propose a goal ramp?".into(),
        streaming: false,
    }));

    // User reply
    log.push(Cell::UserPrompt(cells::UserPrompt {
        text: "yes".into(),
        at: chrono::Local::now(),
    }));

    // Render via raw stdout (no Ratatui terminal needed for static dump).
    let lines = log.render(80, 30);
    for ln in lines {
        println!("{}", flatten(&ln));
    }
}

fn flatten(ln: &ratatui::text::Line<'_>) -> String {
    ln.spans.iter().map(|s| s.content.as_ref()).collect()
}