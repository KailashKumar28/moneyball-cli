//! What the refresh cell draws for each way a run can end (the verdict's
//! words are tested in moneyball_core::refresh).

use super::*;
use moneyball_core::refresh::CANCELLED;
use moneyball_core::schema::{Trigger, RUN_SCHEMA};

fn step(name: &str, status: StepStatus) -> RunStep {
    RunStep {
        name: name.into(),
        status,
        ms: 1200,
        counts: Default::default(),
        error_kind: None,
        message: None,
        fix: None,
        reused: false,
    }
}

fn finished(outcome: Outcome, steps: Vec<RunStep>) -> RefreshCell {
    let mut cell = RefreshCell::new("Thu 08 Oct".into(), false);
    for s in &steps {
        cell.set(&s.name.clone(), Row::Done(s.clone()));
    }
    cell.end = Some(End::Finished {
        run: Box::new(RunRecord {
            schema: RUN_SCHEMA.into(),
            workspace_id: "ws".into(),
            date: "2026-10-09".into(),
            trigger: Trigger::Manual,
            started_at: String::new(),
            finished_at: String::new(),
            outcome,
            steps,
            artifacts: Default::default(),
        }),
        ms: 2000,
        numbers: None,
        report: None,
        last_good: Some("27 Sep".into()),
    });
    cell
}

fn text(cell: &RefreshCell) -> String {
    cell.display_lines(140)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_run_that_never_started_shows_no_steps() {
    let mut cell = RefreshCell::new("Thu 08 Oct".into(), true);
    cell.end = Some(End::CouldNotStart("a refresh is already running".into()));
    let t = text(&cell);
    assert!(t.contains("could not start") && t.contains("already running"));
    assert!(
        !t.contains("waits for") && !t.contains("Meta insights"),
        "{}",
        t
    );
}

#[test]
fn cancel_crash_and_carry_over_say_what_happened() {
    let mut crm = step("crm", StepStatus::Skipped);
    crm.error_kind = Some(CANCELLED.into());
    crm.message = Some("cancelled".into());
    let t = text(&finished(
        Outcome::Degraded,
        vec![
            step("meta", StepStatus::Done),
            step("assets", StepStatus::Done),
            crm,
        ],
    ));
    assert!(
        t.contains("cancelled")
            && t.contains(
                "! Cancelled after step 2. What finished is kept; /refresh runs the rest."
            ),
        "{}",
        t
    );

    let mut meta = step("meta", StepStatus::Done);
    meta.reused = true;
    assert!(text(&finished(Outcome::Ok, vec![meta])).contains(REUSED_NOTE));

    let mut cell = RefreshCell::new("Thu 08 Oct".into(), false);
    cell.set("meta", Row::Done(step("meta", StepStatus::Done)));
    cell.end = Some(End::Stopped(
        "its run record keeps the steps that finished".into(),
    ));
    let t = text(&cell);
    assert!(
        t.contains("stopped unexpectedly") && t.contains("Meta insights"),
        "{}",
        t
    );
}
