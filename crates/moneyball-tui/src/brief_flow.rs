//! /brief: the deterministic portfolio numbers as a tool cell, then the
//! configured LLM's reading of them. Split from commands.rs (size cap);
//! the agent's `brief` tool prints the same lines.

use crate::agent_turn::{call_agent, system_prompt};
use crate::*;
use moneyball_core::agent::prompt::BRIEF_SYSTEM_PROMPT;
use moneyball_core::agent::tool_text::{brief_text, format_brief_as_lines};

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
        let sys = system_prompt(app, BRIEF_SYSTEM_PROMPT);
        let turn = app
            .session
            .typed_tool_turn("/brief", "brief", serde_json::json!({}), output);
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
