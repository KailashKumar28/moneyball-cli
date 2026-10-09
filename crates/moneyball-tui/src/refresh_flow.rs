//! /refresh: the daily refresh inside the TUI. A worker runs
//! refresh::run and sends an event as each step starts and finishes, so
//! the cell grows line by line (Meta, creatives and leads, CRM, report)
//! instead of sitting on a spinner; the end adds the outcome, the fix and
//! the report. /fetch is its old name, and /brief self-heals through it.

use crate::chat::{cells, Cell};
use crate::*;
use moneyball_core::refresh::{self, Options};
use moneyball_core::schema::{Outcome, RunRecord, StepStatus, Trigger};

pub(crate) fn run_refresh(app: &mut App, force: bool, then_brief: bool) {
    if app.stream.is_some() {
        app.status = Some("still working - esc to stop waiting, then resend".into());
        return;
    }
    app.chat.push(Cell::ToolCall(cells::ToolCall {
        name: "refresh".into(),
        args: format!("for {}", refresh::data_day_label()),
        status: cells::ToolStatus::Running,
    }));
    app.chat.push(Cell::ToolResult(cells::ToolResult {
        name: "refresh".into(),
        output: Vec::new(),
        success: true,
        duration_ms: 0,
    }));
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let cfg = app.cfg.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        // The token is resolved at the edge (core never reads it itself).
        let token = moneyball_core::secrets::load_meta_token();
        let opts = Options {
            trigger: Trigger::Manual,
            force,
            days: 28,
        };
        let step_tx = tx.clone();
        let mut n = 0;
        let result = refresh::run(&cfg, token.as_deref(), &opts, &mut |p| {
            let line = match p {
                refresh::Progress::Begin(name) => refresh::running_line(n + 1, name),
                refresh::Progress::Done(step) => {
                    n += 1;
                    refresh::step_line(n, step)
                }
            };
            let _ = step_tx.send(StreamEvent::RefreshStep(line));
        });
        let ms = started.elapsed().as_millis() as u64;
        let _ = tx.send(match result {
            Ok(run) => StreamEvent::RefreshDone {
                run: Box::new(run),
                ms,
                then_brief,
            },
            Err(e) => StreamEvent::RefreshFailed {
                err: e.to_string(),
                ms,
            },
        });
    });
    app.stream = Some(rx);
}

pub(crate) fn on_refresh_step(app: &mut App, line: String) {
    let Some(cell) = app.chat.tool_result_mut("refresh") else {
        return;
    };
    // A finished step replaces its own "running" line.
    let number = line.split_whitespace().next();
    if cell
        .output
        .last()
        .is_some_and(|l| l.ends_with("running") && l.split_whitespace().next() == number)
    {
        cell.output.pop();
    }
    cell.output
        .extend(line.lines().map(|l| l.trim_start().to_string()));
}

pub(crate) fn on_refresh_done(app: &mut App, run: RunRecord, ms: u64, then_brief: bool) {
    app.chat.finalize_tool(run.outcome != Outcome::Failed);
    let mut tail = vec![String::new()];
    tail.push(match run.outcome {
        Outcome::Ok => "outcome: ok".to_string(),
        Outcome::Failed => {
            "outcome: failed - nothing new was written; your last good data is untouched".into()
        }
        _ => {
            let missing: Vec<&str> = run
                .steps
                .iter()
                .filter(|s| matches!(s.status, StepStatus::Failed | StepStatus::Partial))
                .map(|s| s.name.as_str())
                .collect();
            format!("outcome: degraded ({})", missing.join(", "))
        }
    });
    for fix in run.steps.iter().filter_map(|s| s.fix.as_ref()) {
        tail.push(format!("fix: {}  (in a shell), then /refresh", fix));
    }
    if let Some(report) = run.artifacts.get("report") {
        tail.push(format!("report: {}", report));
    }
    if let Some(cell) = app.chat.tool_result_mut("refresh") {
        cell.duration_ms = ms;
        cell.output.extend(tail);
    }
    // The bar and the brief read the new snapshot from here on.
    app.load_brief();
    // Only with a snapshot, so a failed pull never loops back into /brief.
    if then_brief && app.brief.is_some() {
        crate::commands::run_brief(app);
    }
}

pub(crate) fn on_refresh_failed(app: &mut App, err: String, ms: u64) {
    app.chat.finalize_tool(false);
    if let Some(cell) = app.chat.tool_result_mut("refresh") {
        cell.duration_ms = ms;
        cell.output.push(err.clone());
    }
    // Failed tool cells are never the last word (ARCHITECTURE section 5).
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: format!("the refresh could not start: {}", err),
        streaming: false,
    }));
}
