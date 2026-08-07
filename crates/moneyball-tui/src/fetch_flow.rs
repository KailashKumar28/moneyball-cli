//! /fetch flow - worker-thread Meta pull + completion handlers that
//! chain into the brief commentary. Split from commands.rs (size cap).

use crate::commands::{
    app_state_block, build_brief_prompt, call_agent, format_brief_as_lines, BRIEF_SYSTEM_PROMPT,
};
use crate::*;

/// Pull `days` of insights from Meta on a worker thread (the network pull
/// takes seconds - blocking the event loop here froze the UI). The result
/// arrives as StreamEvent::FetchDone/FetchFailed on the tick drain, which
/// hands it to `on_fetch_done`/`on_fetch_failed` below. Shared by /fetch
/// and by /brief's self-heal path when no snapshot exists yet.
pub(crate) fn run_fetch(app: &mut App, days: u32) {
    use crate::chat::cells;
    use crate::chat::Cell;
    if app.stream.is_some() {
        app.status = Some("still working - esc to interrupt, then resend".into());
        return;
    }
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: format!(
            "fetching {} days of insights from Meta (this can take a moment)...",
            days
        ),
        streaming: false,
    }));
    // Token resolved at the edge (core takes it as a parameter - the
    // no-ambient-state hedge in docs/CLOUD_PLAN.md).
    let Some(token) = moneyball_core::secrets::load_meta_token() else {
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: "no Meta token yet - run /setup to connect Meta, then /fetch.".into(),
            streaming: false,
        }));
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let cfg = app.cfg.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let ev = match moneyball_core::fetch::fetch_snapshot(&cfg, &token, days) {
            Ok(report) => StreamEvent::FetchDone {
                report,
                days,
                ms: started.elapsed().as_millis() as u64,
            },
            Err(e) => StreamEvent::FetchFailed {
                err: format!("{}", e),
                days,
                ms: started.elapsed().as_millis() as u64,
            },
        };
        let _ = tx.send(ev);
    });
    app.stream = Some(rx);
}

/// Fetch worker succeeded: show the per-product rows, then load the fresh
/// snapshot and chain into the brief + streaming LLM commentary. Called
/// from the event loop's drain AFTER it cleared `app.stream`, so
/// `call_agent` is free to start the LLM stream.
pub(crate) fn on_fetch_done(
    app: &mut App,
    report: moneyball_core::fetch::FetchReport,
    days: u32,
    ms: u64,
) {
    let mut out: Vec<String> = report
        .per_product
        .iter()
        .map(|(name, n)| format!("{:<40} {:>5} rows", name, n))
        .collect();
    out.push(String::new());
    match &report.creatives_error {
        None => out.push(format!(
            "creatives captured: {} ({} images cached, {} downloaded)",
            report.creatives, report.assets, report.assets_downloaded
        )),
        Some(e) => out.push(format!(
            "warn: creatives capture failed ({}) - snapshot still ok",
            e
        )),
    }
    out.push(format!("snapshot written: {}", report.path.display()));
    app.chat
        .push_tool("fetch", &format!("{} days", days), out, true, ms);
    app.load_brief();
    if let Some(b) = &app.brief {
        let lines = format_brief_as_lines(b);
        let user_prompt = build_brief_prompt(b);
        app.chat.push_tool("brief", "", lines, true, 0);
        let sys = format!("{}\n\n{}", BRIEF_SYSTEM_PROMPT, app_state_block(app));
        call_agent(app, &sys, &user_prompt);
    }
}

pub(crate) fn on_fetch_failed(app: &mut App, err: String, days: u32, ms: u64) {
    app.chat
        .push_tool("fetch", &format!("{} days", days), vec![err], false, ms);
}
