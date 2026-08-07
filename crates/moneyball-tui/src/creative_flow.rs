//! /report flow (slice C) - generate the creative report on a worker
//! thread (image re-encoding takes a second or two; the event loop
//! never blocks) and surface the text summary + browser path as a
//! tool cell. Same worker/drain shape as fetch_flow.rs.

use crate::chat::{cells, Cell};
use crate::*;

pub(crate) fn run_report(app: &mut App, arg: &str) {
    if app.stream.is_some() {
        app.status = Some("still working - esc to interrupt, then resend".into());
        return;
    }
    // Optional arg: trailing window in days ("/report 7"); default 1.
    let window: u32 = arg.trim().parse().unwrap_or(1);
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: format!(
            "building the creative report ({}d window) from the latest snapshot...",
            window.max(1)
        ),
        streaming: false,
    }));
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let cfg = app.cfg.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let ev = match moneyball_core::report::generate(&cfg, None, window) {
            Ok(out) => StreamEvent::ReportDone {
                out: Box::new(out),
                ms: started.elapsed().as_millis() as u64,
            },
            Err(e) => StreamEvent::ReportFailed {
                err: format!("{}", e),
                ms: started.elapsed().as_millis() as u64,
            },
        };
        let _ = tx.send(ev);
    });
    app.stream = Some(rx);
}

pub(crate) fn on_report_done(
    app: &mut App,
    out: Box<moneyball_core::report::ReportOutput>,
    ms: u64,
) {
    let mut lines: Vec<String> = moneyball_core::report::text_summary(&out.report)
        .lines()
        .map(String::from)
        .collect();
    if !out.report.source.crm_present {
        lines.push("warn: no CRM data - L/Q/V/B are zeros, not truths.".into());
    }
    lines.push(String::new());
    lines.push(format!("open in browser: {}", out.html_path.display()));
    app.chat.push_tool("report", "", lines, true, ms);
}

pub(crate) fn on_report_failed(app: &mut App, err: String, ms: u64) {
    app.chat
        .push_tool("report", "", vec![err.clone()], false, ms);
    // Failed tool cells are never the last word (ARCHITECTURE section 5).
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: format!(
            "couldn't build the report: {}. /fetch pulls a fresh snapshot if none exists yet.",
            err
        ),
        streaming: false,
    }));
}
