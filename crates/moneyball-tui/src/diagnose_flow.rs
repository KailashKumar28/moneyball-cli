//! /diagnose: the checks for one product as a tool cell, then the
//! advisor's reading of them. The agent's `diagnose` tool returns the
//! same text.

use crate::commands::{app_state_block, call_agent, typed_tool_turn};
use crate::*;
use moneyball_core::diagnose;

const DIAGNOSE_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads. \
The user ran /diagnose: the diagnose tool's result holds four checks for one product - where \
its CRM leads die, which ads are tiring, the last 7 days against the days before, and what \
changed in its ad account. Name the most likely cause of weak results and the one lever to \
pull first (the lead form, phone data, sales follow-up, swapping a tired ad, undoing a \
change), citing the numbers. A form, data or sales problem is not fixed by cutting ad spend - \
say so when it applies. If CRM data is missing, say which checks that leaves blind and its \
fix. 4-8 sentences, no preamble.";

/// What the diagnose tool returns: the four checks, and the CRM state
/// the model must not read past.
pub(crate) fn diagnose_output(
    cfg: &moneyball_core::AppConfig,
    snap: &moneyball_core::Snapshot,
    product: &str,
) -> String {
    let mut out = diagnose::text(&diagnose::compute(snap, cfg, product));
    if let Some(note) = moneyball_core::crm::status::CrmHealth::of(cfg, snap).note() {
        out.push_str(&format!("IMPORTANT: {}\n", note));
    }
    out
}

pub(crate) fn run_diagnose(app: &mut App, arg: &str) {
    let started = std::time::Instant::now();
    let Ok(product) = crate::funnel_flow::resolve(&app.cfg, arg) else {
        let products: Vec<&str> = app
            .cfg
            .workspace
            .as_ref()
            .map(|w| w.products.iter().map(|p| p.name.as_str()).collect())
            .unwrap_or_default();
        app.chat.note(format!(
            "usage: /diagnose <product>   configured products: {}",
            if products.is_empty() {
                "(none - run /setup)".to_string()
            } else {
                products.join(", ")
            }
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
    let mut lines: Vec<String> = diagnose::text(&diagnose::compute(&snap, &app.cfg, &product))
        .lines()
        .map(String::from)
        .collect();
    if let Some(note) = moneyball_core::crm::status::CrmHealth::of(&app.cfg, &snap).note() {
        lines.push(format!("warn: {}", note));
    }
    app.chat.push_tool(
        "diagnose",
        arg,
        lines,
        true,
        started.elapsed().as_millis() as u64,
    );
    let sys = format!("{}\n\n{}", DIAGNOSE_SYSTEM_PROMPT, app_state_block(app));
    let output = diagnose_output(&app.cfg, &snap, &product);
    let args = serde_json::json!({ "product": product });
    let turn = typed_tool_turn(app, &format!("/diagnose {}", arg), "diagnose", args, output);
    call_agent(app, &sys, turn);
}
