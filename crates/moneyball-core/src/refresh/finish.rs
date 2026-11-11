//! A finished run in words: the header's few ("finished without CRM"),
//! the verdict sentence under the steps, and the commands that follow it.
//! Unstyled: the front-end maps `Tint` to a colour and lays it out.

use serde::Serialize;

use super::CANCELLED;
use crate::schema::{Outcome, RunRecord, RunStep, StepStatus};
use crate::words::{Command, Tint};

/// What a second /refresh does once today's Meta pull is in.
pub(crate) const REUSES_META: &str = "reuses today's Meta pull: only CRM and the report run again";

/// The first sentence after a run that did not go cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunVerdict {
    pub tint: Tint,
    /// What happened.
    pub headline: String,
    /// What it means for the numbers; may be empty.
    pub rest: String,
    /// The command that finishes the job, when one does.
    pub next: Option<Command>,
}

pub fn cancelled(s: &RunStep) -> bool {
    s.error_kind.as_deref() == Some(CANCELLED)
}

fn step<'a>(run: &'a RunRecord, name: &str) -> Option<&'a RunStep> {
    run.steps.iter().find(|s| s.name == name)
}

fn no_crm(run: &RunRecord) -> bool {
    step(run, "report").is_some_and(|s| s.error_kind.as_deref() == Some("no_crm"))
}

/// The header's few words for a run that did not go cleanly: where a
/// failed one stopped, what a degraded one is missing.
pub fn gaps(run: &RunRecord) -> String {
    if run.outcome == Outcome::Failed {
        let failed = |s: &RunStep| s.status == StepStatus::Failed;
        let at = run.steps.iter().position(failed).map_or(1, |i| i + 1);
        return format!("stopped at step {}", at);
    }
    if run.steps.iter().any(cancelled) {
        return "cancelled".into();
    }
    if no_crm(run) || step(run, "crm").is_some_and(|s| s.status == StepStatus::Failed) {
        return "finished without CRM".into();
    }
    match step(run, "meta").and_then(|s| s.counts.get("products_failed")) {
        Some(1) => "one product missing".into(),
        Some(n) => format!("{} products missing", n),
        None => "finished with gaps".into(),
    }
}

/// What happened, then what it means for the numbers. None for a clean
/// run. `last_good`: the data day on screen before the run; `day`: the
/// day this run fills.
pub fn verdict(run: &RunRecord, last_good: Option<&str>, day: &str) -> Option<RunVerdict> {
    match run.outcome {
        Outcome::Ok | Outcome::Running => None,
        Outcome::Failed => Some(RunVerdict {
            tint: Tint::Bad,
            headline: "Nothing was written today.".into(),
            rest: last_good
                .map(|d| format!("Your last good data ({}) is untouched.", d))
                .unwrap_or_default(),
            next: None,
        }),
        Outcome::Degraded => {
            let (headline, rest, next) = degraded(run, day);
            Some(RunVerdict {
                tint: Tint::Warn,
                headline,
                rest: rest.into(),
                next,
            })
        }
    }
}

fn degraded(run: &RunRecord, day: &str) -> (String, &'static str, Option<Command>) {
    let status = |name: &str, st: StepStatus| step(run, name).is_some_and(|s| s.status == st);
    if let Some(at) = run.steps.iter().position(cancelled) {
        (
            format!("Cancelled after step {}.", at),
            "What finished is kept.",
            Some(Command::new("/refresh", "runs the rest")),
        )
    } else if no_crm(run) {
        (
            "The report went out without CRM.".into(),
            // "Complete" only when it is: a 4-of-5 pull sits above.
            if status("meta", StepStatus::Done) {
                "Meta numbers are complete; it leaves the CRM figures out and withholds verdicts."
            } else {
                "It leaves the CRM figures out and withholds verdicts."
            },
            None,
        )
    } else if status("crm", StepStatus::Failed) {
        (
            "CRM was not refreshed.".into(),
            "The report uses the CRM data this snapshot already had.",
            None,
        )
    } else if status("meta", StepStatus::Partial) {
        (
            format!("Some products have no data for {}.", day),
            "The others are complete.",
            None,
        )
    } else if status("report", StepStatus::Failed) {
        (
            "No report was written.".into(),
            "The data is in.",
            Some(Command::new("/report", "builds one from it")),
        )
    } else {
        (
            "Some creatives, leads or ad sets were not captured.".into(),
            "Spend and Meta leads are complete.",
            None,
        )
    }
}

/// The fixes a run's steps name, each with what it does.
pub fn fixes(run: &RunRecord) -> Vec<Command> {
    let named = run.steps.iter().filter_map(|s| s.fix.as_deref());
    named.map(Command::fix).collect()
}

/// The refresh to run once the fixes are done; it says what it reuses
/// when this run's Meta pull came in.
pub fn rerun(run: &RunRecord) -> Command {
    let meta_in = step(run, "meta").is_some_and(|s| s.status != StepStatus::Failed);
    Command::new("/refresh", if meta_in { REUSES_META } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Trigger, RUN_SCHEMA};

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

    fn run(outcome: Outcome, steps: Vec<RunStep>) -> RunRecord {
        RunRecord {
            schema: RUN_SCHEMA.into(),
            workspace_id: "ws".into(),
            date: "2026-10-09".into(),
            trigger: Trigger::Manual,
            started_at: String::new(),
            finished_at: String::new(),
            outcome,
            steps,
            artifacts: Default::default(),
        }
    }

    fn degraded(steps: Vec<RunStep>) -> RunVerdict {
        verdict(&run(Outcome::Degraded, steps), Some("27 Sep"), "Thu 08 Oct").unwrap()
    }

    #[test]
    fn no_crm_says_meta_is_complete_only_when_it_is() {
        let mut meta = step("meta", StepStatus::Partial);
        meta.counts = [("products".into(), 4), ("products_failed".into(), 1)].into();
        let mut report = step("report", StepStatus::Partial);
        report.error_kind = Some("no_crm".into());
        let part = degraded(vec![meta, report.clone()]);
        assert_eq!(part.headline, "The report went out without CRM.");
        assert!(!part.rest.contains("Meta numbers are complete"));
        let whole = degraded(vec![step("meta", StepStatus::Done), report.clone()]);
        assert!(whole.rest.starts_with("Meta numbers are complete"));
        assert_eq!(
            gaps(&run(Outcome::Degraded, vec![report])),
            "finished without CRM"
        );
    }

    #[test]
    fn a_cancelled_run_names_the_command_that_runs_the_rest() {
        let mut crm = step("crm", StepStatus::Skipped);
        crm.error_kind = Some(CANCELLED.into());
        let steps = vec![
            step("meta", StepStatus::Done),
            step("assets", StepStatus::Done),
            crm,
        ];
        assert_eq!(gaps(&run(Outcome::Degraded, steps.clone())), "cancelled");
        let v = degraded(steps);
        assert_eq!(v.tint, Tint::Warn);
        assert_eq!(v.headline, "Cancelled after step 2.");
        assert_eq!(v.next, Some(Command::new("/refresh", "runs the rest")));
    }

    #[test]
    fn a_failed_run_keeps_the_last_good_day_and_its_fix_leads_to_a_rerun() {
        let mut meta = step("meta", StepStatus::Failed);
        meta.fix = Some("/setup meta".into());
        let failed = run(Outcome::Failed, vec![meta]);
        let v = verdict(&failed, Some("27 Sep"), "Thu 08 Oct").unwrap();
        assert_eq!(v.tint, Tint::Bad);
        assert_eq!(v.rest, "Your last good data (27 Sep) is untouched.");
        assert_eq!(fixes(&failed), vec![Command::fix("/setup meta")]);
        assert_eq!(rerun(&failed).does, "");
        assert_eq!(gaps(&failed), "stopped at step 1");
        let mut crm = step("crm", StepStatus::Failed);
        crm.fix = Some("/connect crm".into());
        let later = run(Outcome::Degraded, vec![step("meta", StepStatus::Done), crm]);
        assert_eq!(rerun(&later).does, REUSES_META);
        assert_eq!(verdict(&run(Outcome::Ok, vec![]), None, ""), None);
    }
}
