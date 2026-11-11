//! The refresh forwarder against a hand-fed channel: no network, no
//! worker. What reaches the interface, and what a reload would read.

use std::sync::mpsc::channel;
use std::sync::Mutex as StdMutex;

use moneyball_core::schema::{RunRecord, RunStep};

use super::*;
use crate::run_view::StepState;

fn in_flight() -> Mutex<RefreshRun> {
    Mutex::new(RefreshRun::begin(Arc::default()))
}

fn step(name: &str, status: &str) -> RunStep {
    serde_json::from_value(serde_json::json!({"name": name, "status": status, "ms": 900})).unwrap()
}

fn run_with(events: Vec<Ev>, shared: &Mutex<RefreshRun>) -> Vec<serde_json::Value> {
    let (tx, rx) = channel();
    for ev in events {
        tx.send(ev).unwrap();
    }
    drop(tx);
    let seen = StdMutex::new(Vec::new());
    forward(rx, shared, Some("07 Oct".into()), |ev| {
        // The run is over by the time its last event is seen.
        let last = !matches!(ev, RefreshEvent::Begin { .. } | RefreshEvent::Done { .. });
        assert_eq!(lock(shared).running, !last);
        lock(&seen).push(serde_json::to_value(&ev).unwrap());
        true
    });
    seen.into_inner().unwrap()
}

#[test]
fn steps_update_the_state_and_a_finished_run_ends_it() {
    let shared = in_flight();
    let run: RunRecord = serde_json::from_value(serde_json::json!({
        "schema": "moneyball.run/1", "workspace_id": "w", "date": "2026-10-09",
        "trigger": "manual", "started_at": "", "outcome": "failed",
        "steps": [{"name": "meta", "status": "failed", "ms": 900, "fix": "/setup meta",
                   "message": "Meta refused the token"}],
    }))
    .unwrap();
    let seen = run_with(
        vec![
            Ev::Begin { step: "meta" },
            Ev::Done(Box::new(step("meta", "failed"))),
            Ev::Finished {
                run: Box::new(run),
                ms: 81_000,
            },
        ],
        &shared,
    );
    let types: Vec<&str> = seen.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["begin", "done", "finished"]);
    assert_eq!(seen[0]["step"]["status"], "running");
    let end = &seen[2];
    assert_eq!(end["time"], "1m 21s");
    assert_eq!(end["verdict"]["headline"], "Nothing was written today.");
    assert_eq!(
        end["verdict"]["rest"],
        "Your last good data (07 Oct) is untouched."
    );
    assert_eq!(end["fixes"][0]["kind"], "terminal");
    assert_eq!(end["rerun"]["kind"], "refresh");
    assert!(end["numbers"].is_null() && end["report_path"].is_null());
    let state = state_view(&lock(&shared), None);
    assert!(!state.running && state.started_ms_ago.is_none());
    assert_eq!(state.steps[0].status, StepState::Failed);
    assert_eq!(state.steps[1].status, StepState::Waiting);
}

#[test]
fn a_worker_that_dies_says_so_and_frees_the_state() {
    let shared = in_flight();
    let seen = run_with(vec![Ev::Begin { step: "meta" }], &shared);
    assert_eq!(seen[1]["type"], "died");
    assert!(seen[1]["message"]
        .as_str()
        .unwrap()
        .contains("stopped unexpectedly"));
    assert!(!lock(&shared).running);
    let refused = run_with(
        vec![Ev::CouldNotStart {
            error: "a refresh is already running".into(),
        }],
        &in_flight(),
    );
    assert_eq!(refused.len(), 1, "could_not_start is final: no died after");
    assert_eq!(refused[0]["error"]["kind"], "could_not_start");
}

#[test]
fn a_second_refresh_is_refused_while_one_runs_and_idle_state_is_empty() {
    let shared = Arc::new(in_flight());
    let cfg = AppConfig::resolve_optional(Some("/nonexistent/mb-desktop-refresh"), None);
    let err = start(cfg, shared, false, |_| true).unwrap_err();
    assert_eq!(err.kind, "busy");
    // Before any run in this launch: nothing, or the last recorded run.
    let idle = state_view(&RefreshRun::default(), None);
    assert!(!idle.running && idle.day.is_empty() && idle.steps.is_empty());
}
