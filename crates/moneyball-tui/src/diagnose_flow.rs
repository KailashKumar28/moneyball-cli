//! /diagnose: the checks for one product as a tool cell, then the
//! advisor's reading of them. The agent's `diagnose` tool returns the
//! same text.

use crate::agent_turn::{call_agent, system_prompt};
use crate::*;
use moneyball_core::agent::prompt::DIAGNOSE_SYSTEM_PROMPT;
use moneyball_core::agent::tool_text::diagnose_output;

pub(crate) fn run_diagnose(app: &mut App, arg: &str) {
    let started = std::time::Instant::now();
    let Ok(product) = moneyball_core::product::resolve_configured(&app.cfg, arg) else {
        let products: Vec<&str> = app
            .cfg
            .workspace
            .as_ref()
            .map(|w| w.products.iter().map(|p| p.name.as_str()).collect())
            .unwrap_or_default();
        app.chat.note(format!(
            "usage: /diagnose <product>   configured products: {}",
            products.join(", ")
        ));
        return;
    };
    let snap = match app
        .cfg
        .snap_for(app.cfg.date.as_deref())
        .and_then(|p| moneyball_core::snapshot::load(&p))
    {
        Ok(s) => s,
        Err(_) => {
            app.chat
                .note("no snapshot yet - run /refresh first (or /brief, which self-heals).");
            return;
        }
    };
    // The cell shows exactly what the advisor reads.
    let output = diagnose_output(&app.cfg, &snap, &product, &crate::agent_turn::say);
    let lines = output.lines().map(String::from).collect();
    app.chat.push_tool(
        "diagnose",
        arg,
        lines,
        true,
        started.elapsed().as_millis() as u64,
    );
    let sys = system_prompt(app, DIAGNOSE_SYSTEM_PROMPT);
    let args = serde_json::json!({ "product": product });
    let turn = app
        .session
        .typed_tool_turn(&format!("/diagnose {}", arg), "diagnose", args, output);
    call_agent(app, &sys, turn);
}
