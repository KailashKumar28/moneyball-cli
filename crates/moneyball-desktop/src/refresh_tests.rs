//! The refresh forwarder against a hand-fed channel: no network, no
//! worker. What reaches the interface, and what a reload would read:
//! the run in flight while there is one, else the workspace's record.

use std::sync::mpsc::channel;
use std::sync::Mutex as StdMutex;

use moneyball_core::schema::{RunRecord, RunStep};

use super::*;
use crate::fixtures::demo;
use crate::run_view::StepState;

fn in_flight() -> Mutex<RefreshRun> {
    Mutex::new(RefreshRun::begin(Arc::default()))
}

fn step(name: &str, status: &str) -> RunStep {
    serde_json::from_value(serde_json::json!({"name": name, "status": status, "ms": 900})).unwrap()
}

/// A run that failed at Meta, as core records it.
fn failed_run(date: &str) -> RunRecord {
    serde_json::from_value(serde_json::json!({
        "schema": "moneyball.run/1", "workspace_id": "w", "date": date,
        "trigger": "manual", "started_at": "2026-10-09T02:00:00Z",
        "finished_at": "2026-10-09T02:01:21Z", "outcome": "failed",
        "steps": [{"name": "meta", "status": "failed", "ms": 900, "fix": "/setup meta",
                   "message": "Meta refused the token"}],
    }))
    .unwrap()
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
        if !last {
            let state = state_view(shared, None);
            assert!(state.running && state.finished.is_none());
            assert_eq!(state.steps.len(), 4);
        }
        lock(&seen).push(serde_json::to_value(&ev).unwrap());
        true
    });
    seen.into_inner().unwrap()
}

#[test]
fn steps_update_the_state_and_a_finished_run_ends_it() {
    let shared = in_flight();
    let seen = run_with(
        vec![
            Ev::Begin { step: "meta" },
            Ev::Done(Box::new(step("meta", "failed"))),
            Ev::Finished {
                run: Box::new(failed_run("2026-10-09")),
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
    assert_eq!(end["run"]["gaps"], "stopped at step 1");
    assert_eq!(end["verdict"]["headline"], "Nothing was written today.");
    assert_eq!(
        end["verdict"]["rest"],
        "Your last good data (07 Oct) is untouched."
    );
    assert_eq!(end["fixes"][0]["kind"], "terminal");
    assert_eq!(end["rerun"]["kind"], "refresh");
    assert!(end["numbers"].is_null() && end["report_path"].is_null());
    // Over: with no workspace record to read, there is nothing to show.
    let state = state_view(&shared, None);
    assert!(!state.running && state.steps.is_empty() && state.finished.is_none());
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
    let err = start(cfg.clone(), shared, false, |_| true).unwrap_err();
    assert_eq!(err.kind, "busy");
    // No run in flight and none recorded: nothing.
    let idle = state_view(&Mutex::default(), Some(&cfg));
    assert!(!idle.running && idle.day.is_empty() && idle.steps.is_empty());
    assert!(idle.finished.is_none());
}

/// The review's three: another workspace's run, a refresh that could
/// not start, and a later run by the terminal or the schedule.
#[test]
fn at_rest_the_state_is_the_open_workspaces_last_recorded_run_in_full() {
    let a = demo::workspace("state-a", true, true);
    let b = demo::workspace("state-b", true, false);
    let clean = demo::run_record(&a, true);
    runs::write(&a, &clean).unwrap();
    // This launch ran a refresh in A; it is over.
    let shared = in_flight();
    let finish = Ev::Finished {
        run: Box::new(clean.clone()),
        ms: recorded_ms(&clean),
    };
    let live = run_with(vec![finish], &shared).remove(0);

    // A reload reads what the live event said, from the record alone.
    let at_rest = state_view(&shared, Some(&a));
    assert_eq!(at_rest.day, "Thu 08 Oct");
    assert_eq!(at_rest.steps[0].status, StepState::Done);
    let mut read = serde_json::to_value(at_rest.finished.as_ref().unwrap()).unwrap();
    read["type"] = "finished".into();
    assert_eq!(read, live);
    assert_eq!(read["time"], "1m 21s");
    assert!(read["numbers"].is_array() && read["report_path"].is_string());

    // B has never run: A's run is not shown under B's name.
    let other = state_view(&shared, Some(&b));
    assert!(other.steps.is_empty() && other.finished.is_none());

    // A refresh that could not start leaves the recorded run on screen.
    *lock(&shared) = RefreshRun::begin(Arc::default());
    let refused = Ev::CouldNotStart {
        error: "a refresh is already running".into(),
    };
    run_with(vec![refused], &shared);
    assert_eq!(
        state_view(&shared, Some(&a)).steps[0].status,
        StepState::Done
    );

    // The schedule ran later and failed: that is the run shown now.
    runs::write(&a, &failed_run("2026-10-10")).unwrap();
    let later = state_view(&shared, Some(&a));
    assert_eq!(later.day, "Fri 09 Oct");
    assert_eq!(later.steps[0].status, StepState::Failed);
    assert_eq!(later.steps[1].status, StepState::Waiting);
    let end = later.finished.unwrap();
    assert_eq!(end.run.gaps, "stopped at step 1");
    let verdict = end.verdict.unwrap();
    assert_eq!(verdict.headline, "Nothing was written today.");
    assert_eq!(verdict.rest, "Your last good data (08 Oct) is untouched.");
    assert!(end.rerun.is_some() && end.numbers.is_none());

    // A run still marked running elsewhere: its steps, no verdict.
    let mut running = failed_run("2026-10-11");
    running.outcome = Outcome::Running;
    runs::write(&a, &running).unwrap();
    let elsewhere = state_view(&shared, Some(&a));
    assert!(!elsewhere.running && elsewhere.finished.is_none());
    assert_eq!(elsewhere.steps.len(), 4);
    for cfg in [a, b] {
        demo::remove(&cfg);
    }
}
