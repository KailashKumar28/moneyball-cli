//! Slash-command surface: the registry, and `submit` dispatching what
//! the user typed (advisor turns start in agent_turn.rs).

use crate::*;

// CLAUDE.md hard rule: only commands with real handlers may appear
// here - this list feeds the palette, /help, AND every LLM system
// prompt (app_state_block).
pub(crate) const COMMANDS: &[(&str, &str)] = &[
    ("/brief", "7-day portfolio brief"),
    (
        "/refresh",
        "pull Meta, then CRM, then write yesterday's report",
    ),
    ("/status", "the last 14 data days and how each refresh went"),
    ("/schedule", "refresh every morning; /schedule off stops it"),
    ("/funnel", "per-entity funnel for a product"),
    (
        "/diagnose",
        "before cutting a product: where leads die, tired ads, trend, changes",
    ),
    (
        "/ledger",
        "the advisor's recommendations and how each came out",
    ),
    (
        "/report",
        "creative report (per-creative funnel) as HTML + summary",
    ),
    (
        "/ask",
        "a question for the advisor (it pulls the numbers it needs)",
    ),
    (
        "/debug",
        "flag this session as buggy for review (optional comment)",
    ),
    (
        "/crm",
        "CRM status: spec, secret, data freshness, next step",
    ),
    ("/snapshot", "list or validate snapshots"),
    ("/clear", "start a fresh conversation (new session file)"),
    (
        "/connect",
        "where ads, leads and the advisor stand; /connect crm connects one",
    ),
    (
        "/setup",
        "re-run the setup wizard; /setup meta replaces the Meta token",
    ),
    ("/help", "list slash commands"),
    ("/quit", "exit moneyball"),
];

pub(crate) fn completions(prefix: &str) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| c.starts_with(prefix))
        .collect()
}

pub(crate) fn submit(app: &mut App) {
    use crate::chat::cells;
    use crate::chat::Cell;
    use chrono::Local;
    // One response at a time: while a stream is in flight, hold new
    // submissions (esc interrupts, like codex).
    if app.stream.is_some() {
        app.status = Some("still responding - esc to interrupt, then resend".into());
        return;
    }
    let line = app.input.trim().to_string();
    // A second /quit confirms; anything else in between disarms it.
    let quit_armed = std::mem::take(&mut app.quit_armed);
    app.input.clear();
    app.cursor = 0;
    app.completions.clear();
    app.completion_idx = None;
    // Clear any stale status hint from a prior command/error so the
    // bottom status line doesn't keep showing the previous result.
    app.status = None;
    if line.is_empty() {
        return;
    }

    // User prompt cell (every input becomes part of the scrollback).
    app.chat.push(Cell::UserPrompt(cells::UserPrompt {
        text: line.clone(),
        at: Local::now(),
    }));

    let mut parts = line.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    match cmd {
        "/quit" | "/exit" | "/q" => {
            if app.refresh.is_some() && !quit_armed {
                app.quit_armed = true;
                app.chat.note(
                    "a refresh is still running - /quit again to stop it and leave \
                     (its run record keeps the steps that finished)",
                );
                return;
            }
            app.chat.note("exiting moneyball.");
            app.quit = true;
        }
        "/brief" => crate::brief_flow::run_brief(app),
        "/status" => crate::status_view::run_status(app),
        "/schedule" => crate::schedule_view::run_schedule(app, arg),
        // /fetch is the old name, kept for muscle memory (unlisted).
        "/refresh" | "/fetch" => crate::refresh_flow::run_refresh(app, arg == "--force", false),
        "/setup" => open_setup(app, arg),
        "/funnel" => crate::funnel_flow::run_funnel(app, arg),
        "/diagnose" => crate::diagnose_flow::run_diagnose(app, arg),
        "/ledger" => crate::ledger_view::run_ledger(app, arg),
        "/debug" => crate::report::run_debug_report(app, arg),
        "/report" => crate::creative_flow::run_report(app, arg),
        "/crm" => crate::crm_flow::run_crm_status(app),
        "/ask" => {
            // /ask is now equivalent to free-form chat (the LLM is the
            // default). Strip the prefix and fall through to the free-
            // form path below.
            let question = if arg.is_empty() {
                line.clone()
            } else {
                arg.to_string()
            };
            crate::agent_turn::run_freeform(app, &question);
        }
        "/snapshot" => list_snapshots(app),
        "/clear" | "/new" => clear_conversation(app),
        // /keychain is the old name, kept for muscle memory (unlisted).
        "/connect" | "/keychain" if arg == "crm" => crate::crm_connect::run(app),
        "/connect" | "/keychain" => crate::connect_view::run_connect(app),
        "/help" | "/?" => show_help(app),
        _ => {
            // A slash typo must never reach the paid LLM path - reject
            // locally and instantly (codex/Claude Code behavior).
            if cmd.starts_with('/') {
                app.chat.push(Cell::System(cells::System(format!(
                    "unknown command {} - type / to see the command list.",
                    cmd
                ))));
                return;
            }
            // Free-form chat: the LLM is the default; slash commands are
            // optional shortcuts. The agent pulls data via its tools.
            crate::agent_turn::run_freeform(app, &line);
        }
    }
}

/// /setup: the wizard prefilled from the saved config (enter keeps each
/// value); /setup meta opens straight at the token.
fn open_setup(app: &mut App, arg: &str) {
    // Prefilled from the saved config, so /setup re-runs as "edit
    // current settings" (Enter keeps values).
    let mut state = match app.cfg.workspace.as_ref() {
        Some(w) => SetupState::prefilled_from(w, &app.cfg.data_root),
        None => SetupState::new(app.cfg.data_root.clone()),
    };
    // `/setup meta`: straight to the token paste, nothing else
    // re-asked (a configured workspace otherwise opens at the model).
    // The saved token shows as bullets: enter re-checks it against
    // every product, typing replaces it.
    let token_only = arg == "meta" && !state.products.is_empty();
    if token_only {
        state.step = 1;
        state.meta_substep = 0;
        state.token_only = true;
        state.meta_input = "\u{2022}".repeat(state.meta_token_len);
    }
    app.chat.note(if token_only {
        "replacing the Meta token: paste the new one, or enter to re-check the saved one."
    } else {
        "opening setup wizard (Enter to keep current values, edit + Enter to change)."
    });
    app.view = View::Setup(state);
}

/// /snapshot: the snapshot dates on disk, and whether the newest loads.
fn list_snapshots(app: &mut App) {
    // List available snapshot dates and validate the latest one
    // parses against the schema.
    let started = std::time::Instant::now();
    let snap_root = app.cfg.snap_dir();
    let dates = moneyball_core::snapshot::list_dates(&snap_root).unwrap_or_default();
    if dates.is_empty() {
        app.chat.push_tool(
            "snapshot",
            "",
            vec![format!(
                "no snapshots in {} - run /refresh to pull one.",
                snap_root.display()
            )],
            false,
            started.elapsed().as_millis() as u64,
        );
    } else {
        let latest = dates.last().cloned().unwrap_or_default();
        let mut out: Vec<String> = vec![format!(
            "{} snapshot(s) in {}:",
            dates.len(),
            snap_root.display()
        )];
        out.extend(dates.iter().map(|d| format!("  {}", d)));
        match moneyball_core::snapshot::load(&snap_root.join(&latest)) {
            Ok(s) => out.push(format!(
                "latest {} validates: {} ads_daily rows",
                latest,
                s.ads_daily.len()
            )),
            Err(e) => out.push(format!("latest {} FAILS validation: {}", latest, e)),
        }
        app.chat.push_tool(
            "snapshot",
            "",
            out,
            true,
            started.elapsed().as_millis() as u64,
        );
    }
}

/// /clear: a fresh history and session file (codex /new), the old one
/// left on disk; the escape hatch when a conversation outgrows the model.
fn clear_conversation(app: &mut App) {
    use crate::chat::cells;
    use crate::chat::Cell;
    // Escape hatch for context growth (codex /new): fresh
    // history + fresh session file. The old session stays on
    // disk untouched (append-only, resumable via -c or --resume).
    if app.session.turn_active() || app.stream.is_some() {
        app.chat
            .note("still responding - esc to interrupt first, then /clear");
        return;
    }
    // Its answers and result land in its cell: keep it until done.
    if app.connect.is_some() {
        app.chat
            .note("a CRM connect is running - finish or esc it, then /clear");
        return;
    }
    // A refresh still running keeps its cell: its outcome and any
    // fix land there.
    app.chat
        .cells
        .retain(|c| matches!(c, Cell::Refresh(r) if r.end.is_none()));
    app.chat.scroll_to_bottom();
    use moneyball_core::agent::Session;
    match moneyball_core::session::SessionLog::create(
        app.cfg.data_root.clone(),
        app.cfg.sessions_root(),
        crate::FRONT_END,
    ) {
        Ok(log) => {
            app.session = Session::new(Some(log), Vec::new());
            app.chat.push(Cell::System(cells::System(
                "fresh conversation - previous session saved on disk (resume with \
                 moneyball --continue)"
                    .into(),
            )));
        }
        Err(e) => {
            app.session = Session::new(None, Vec::new());
            app.chat.push(Cell::System(cells::System(format!(
                "fresh conversation - but the new session log is unavailable ({})",
                e
            ))));
        }
    }
}

/// /help, generated from COMMANDS so the palette, /help and the model's
/// app state cannot drift.
fn show_help(app: &mut App) {
    use crate::chat::cells;
    use crate::chat::Cell;
    // Generated from COMMANDS so the three command surfaces
    // (palette, /help, app_state_block) cannot drift.
    let mut text = String::from("slash commands:");
    for (c, desc) in COMMANDS {
        text.push_str(&format!("\n  {:<10} {}", c, desc));
    }
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text,
        streaming: false,
    }));
}
