//! /funnel flow - product resolution, funnel table cell, and the LLM
//! commentary hand-off. Split from commands.rs (size cap).

use crate::agent_turn::{call_agent, system_prompt};
use crate::*;
use moneyball_core::agent::prompt::FUNNEL_SYSTEM_PROMPT;
use moneyball_core::agent::tool_text::funnel_output;
use moneyball_core::funnel::{self, By};

pub(crate) fn run_funnel(app: &mut App, arg: &str) {
    use crate::chat::cells;
    use crate::chat::Cell;
    let started = std::time::Instant::now();
    // Trailing token may be the level; everything before it is the
    // product (product names contain spaces).
    let (name, by) = match arg.rsplit_once(' ') {
        Some((head, lvl)) if ["campaign", "adset", "ad"].contains(&lvl) => (head.trim(), lvl),
        _ => (arg, "adset"),
    };
    let by = By::parse(by).expect("level pre-validated");
    let Ok(product) = moneyball_core::product::resolve_configured(&app.cfg, name) else {
        let products: Vec<&str> = app
            .cfg
            .workspace
            .as_ref()
            .map(|w| w.products.iter().map(|p| p.name.as_str()).collect())
            .unwrap_or_default();
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: format!(
                "usage: /funnel <product> [campaign|adset|ad]\nconfigured products: {}",
                if products.is_empty() {
                    "(none - run /setup)".into()
                } else {
                    products.join(", ")
                }
            ),
            streaming: false,
        }));
        return;
    };

    let snap = match app
        .cfg
        .snap_for(app.cfg.date.as_deref())
        .and_then(|p| moneyball_core::snapshot::load(&p))
    {
        Ok(s) => s,
        Err(_) => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "no snapshot yet - run /refresh first (or /brief, which self-heals).".into(),
                streaming: false,
            }));
            return;
        }
    };
    // The cell shows the view a person reads; the model gets the full
    // table as the funnel tool's output.
    let rows = funnel::compute(&snap, &app.cfg, &product, 7, by);
    let mut lines = vec![format!(
        "FUNNEL {} - by {} - 7d - snapshot {}",
        product,
        by.name(),
        snap.date
    )];
    lines.extend(funnel::view(&rows).lines().map(String::from));
    lines.extend(funnel::cpl_ranking(&rows));
    if let Some(note) = moneyball_core::crm::status::CrmHealth::of(&app.cfg, &snap).note() {
        lines.push(format!("warn: {}", note));
    }
    app.chat.push_tool(
        "funnel",
        arg,
        lines,
        true,
        started.elapsed().as_millis() as u64,
    );

    let sys = system_prompt(app, FUNNEL_SYSTEM_PROMPT);
    let output = funnel_output(&app.cfg, &snap, &product, by);
    let args = serde_json::json!({ "product": product, "by": by.name() });
    let turn = app
        .session
        .typed_tool_turn(&format!("/funnel {}", arg), "funnel", args, output);
    call_agent(app, &sys, turn);
}
