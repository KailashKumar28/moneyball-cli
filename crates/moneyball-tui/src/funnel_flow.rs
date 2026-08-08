//! /funnel flow - product resolution, funnel table cell, and the LLM
//! commentary hand-off. Split from commands.rs (size cap).

use crate::commands::{app_state_block, call_agent, FUNNEL_SYSTEM_PROMPT};
use crate::*;

pub(crate) fn run_funnel(app: &mut App, arg: &str) {
    use crate::chat::cells;
    use crate::chat::Cell;
    let started = std::time::Instant::now();
    let products: Vec<String> = app
        .cfg
        .workspace
        .as_ref()
        .map(|w| w.products.iter().map(|p| p.name.clone()).collect())
        .unwrap_or_default();

    // Trailing token may be the level; everything before it is the
    // product (product names contain spaces).
    let (product, by) = match arg.rsplit_once(' ') {
        Some((head, lvl)) if ["campaign", "adset", "ad"].contains(&lvl) => {
            (head.trim().to_string(), lvl.to_string())
        }
        _ => (arg.to_string(), "adset".to_string()),
    };
    // Fuzzy resolve: the advisor's shorthand ("Namma Mane") and a
    // user's lowercase both land on the configured name.
    let product = match app.cfg.workspace.as_ref() {
        Some(w) => match moneyball_core::product::resolve_product(&product, &w.products) {
            Ok(name) => name.to_string(),
            Err(_) => product,
        },
        None => product,
    };
    if product.is_empty() || !products.iter().any(|p| p == &product) {
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
    }

    let snap = match app
        .cfg
        .snap_for(app.cfg.date.as_deref())
        .and_then(|p| moneyball_core::snapshot::load(&p))
    {
        Ok(s) => s,
        Err(_) => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "no snapshot yet - run /fetch first (or /brief, which self-heals).".into(),
                streaming: false,
            }));
            return;
        }
    };
    let level = moneyball_core::funnel::By::parse(&by).expect("level pre-validated");
    let rows = moneyball_core::funnel::compute(&snap, &app.cfg, &product, 7, level);
    let mut lines = vec![format!(
        "FUNNEL {} - by {} - 7d - snapshot {}",
        product, by, snap.date
    )];
    lines.extend(
        moneyball_core::funnel::table(&rows)
            .lines()
            .map(String::from),
    );
    if let Some(rank) = moneyball_core::funnel::cpl_ranking(&rows) {
        lines.push(rank);
    }
    let table_text = lines.join("\n");
    app.chat.push_tool(
        "funnel",
        arg,
        lines,
        true,
        started.elapsed().as_millis() as u64,
    );

    let sys = format!("{}\n\n{}", FUNNEL_SYSTEM_PROMPT, app_state_block(app));
    let user = format!(
        "Here is the 7-day per-{} funnel for {} (kill = spend passed the kill table \
         with <=2 qualified; immature = leads still inside the 72h maturation lag):\n\n{}",
        by, product, table_text
    );
    call_agent(app, &sys, &user);
}
