//! /report flow (slice C) - generate the creative report on a worker
//! thread (image re-encoding takes a second or two; the event loop
//! never blocks) and surface the text summary + browser path as a
//! tool cell. Same worker/drain shape as refresh_flow.rs.

use crate::chat::{cells, Cell};
use crate::*;

pub(crate) fn run_report(app: &mut App, arg: &str) {
    if app.stream.is_some() {
        app.chat
            .note("still working on the last answer - esc to interrupt, then resend");
        return;
    }
    // A refresh writes today's report itself; two writers would race.
    if app.refresh.is_some() {
        app.chat
            .note("a refresh is writing today's report - it lands in the refresh above");
        return;
    }
    // Optional arg: trailing window in days ("/report 7"); default 1.
    // Anything else is a typo, never a silent 1-day report.
    let window: u32 = match arg.trim() {
        "" => 1,
        a => match a.parse::<u32>() {
            Ok(n) if n >= 1 => n,
            _ => {
                app.chat.push(Cell::System(cells::System(format!(
                    "usage: /report [days] - e.g. /report 7 (got \"{}\")",
                    a
                ))));
                return;
            }
        },
    };
    // The session's --date pins the snapshot, same as /funnel.
    let date = app.cfg.date.clone();
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: format!(
            "building the creative report ({}d window) from the {} snapshot...",
            window,
            date.as_deref().unwrap_or("latest")
        ),
        streaming: false,
    }));
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let cfg = app.cfg.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let ev = match moneyball_core::report::generate(&cfg, date.as_deref(), window) {
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
        lines.push(
            "no CRM data - the report hides CRM leads, qualified, visits and bookings.".into(),
        );
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
            "couldn't build the report: {}. /refresh pulls a fresh snapshot if none exists yet.",
            err
        ),
        streaming: false,
    }));
}
