//! A refresh run and its four steps as the interface shows them: core's
//! words (refresh/show.rs) for each column, with the step's fix
//! translated. Shared by Today's data-health table and the live refresh.

use chrono::NaiveDate;
use moneyball_core::refresh::{self, STEPS};
use moneyball_core::schema::{Outcome, RunRecord, RunStep, StepStatus, Trigger};
use moneyball_core::words::Command;
use serde::Serialize;

use crate::action::{translate, Action};

/// How the app names a day: "Thu 08 Oct".
pub const DAY: &str = "%a %d %b";

/// Core's `StepStatus` words, plus the two a live run adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Done,
    /// Done for some of it ("4 of 5", "no CRM"): shown as a warning.
    Partial,
    Failed,
    Skipped,
    Waiting,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepView {
    pub name: String,
    pub label: String,
    pub status: StepState,
    pub word: String,
    /// Its counts, or the reason it failed, fell short or was skipped.
    pub detail: String,
    pub time: Option<String>,
    pub reused: bool,
    pub fix: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunView {
    /// The data day, "2026-10-08": the day before the run's snapshot.
    pub date: String,
    pub day: String,
    pub trigger: Trigger,
    pub outcome: Outcome,
    /// What a run that did not go cleanly is missing; empty for a clean one.
    pub gaps: String,
    /// Always the four, in order.
    pub steps: Vec<StepView>,
}

fn blank(name: &str, status: StepState, word: &str) -> StepView {
    StepView {
        name: name.into(),
        label: refresh::step_label(name).into(),
        status,
        word: word.into(),
        detail: String::new(),
        time: None,
        reused: false,
        fix: None,
    }
}

/// A step that has not started, saying what it waits for.
pub fn waiting(name: &str) -> StepView {
    let word = match name {
        "meta" => "waiting",
        "report" => "waits for CRM",
        _ => "waits for Meta",
    };
    blank(name, StepState::Waiting, word)
}

pub fn running(name: &str) -> StepView {
    blank(name, StepState::Running, "running")
}

/// The four steps before any has started.
pub fn all_waiting() -> Vec<StepView> {
    STEPS.iter().map(|name| waiting(name)).collect()
}

/// A finished step. Its counts when it has them to show, else its
/// message: why it failed, fell short or was skipped.
pub fn done(s: &RunStep) -> StepView {
    let no_crm = s.error_kind.as_deref() == Some("no_crm");
    let message = || s.message.clone().unwrap_or_default();
    let (status, detail) = match s.status {
        StepStatus::Done => (StepState::Done, refresh::counts_detail(s)),
        // A report without CRM keeps its counts; its word says "no CRM".
        StepStatus::Partial if no_crm => (StepState::Partial, refresh::counts_detail(s)),
        StepStatus::Partial => (StepState::Partial, message()),
        StepStatus::Failed => (StepState::Failed, message()),
        StepStatus::Skipped => (StepState::Skipped, message()),
    };
    StepView {
        detail,
        time: Some(refresh::duration(s.ms)).filter(|t| !t.is_empty()),
        reused: s.reused && s.status != StepStatus::Skipped,
        fix: s.fix.as_deref().map(|f| translate(&Command::fix(f))),
        ..blank(&s.name, status, &refresh::status_word(s))
    }
}

/// The day before `date` (YYYY-MM-DD) as ("2026-10-08", "Thu 08 Oct");
/// a date that will not parse comes back as it is.
pub fn data_day(date: &str) -> (String, String) {
    match NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.pred_opt())
    {
        Some(d) => (d.format("%Y-%m-%d").to_string(), d.format(DAY).to_string()),
        None => (date.to_string(), date.to_string()),
    }
}

pub fn run_view(run: &RunRecord) -> RunView {
    let (date, day) = data_day(&run.date);
    let gaps = match run.outcome {
        Outcome::Ok | Outcome::Running => String::new(),
        Outcome::Degraded => refresh::gaps(run),
        Outcome::Failed => {
            let failed = run
                .steps
                .iter()
                .position(|s| s.status == StepStatus::Failed);
            format!("stopped at step {}", failed.map_or(1, |i| i + 1))
        }
    };
    let step = |name: &&str| {
        let found = run.steps.iter().find(|s| s.name == **name);
        found.map_or_else(|| waiting(name), done)
    };
    RunView {
        date,
        day,
        trigger: run.trigger,
        outcome: run.outcome,
        gaps,
        steps: STEPS.iter().map(step).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(name: &str, status: StepStatus) -> RunStep {
        serde_json::from_value(serde_json::json!({"name": name, "status": status, "ms": 1200}))
            .unwrap()
    }

    #[test]
    fn a_failed_step_shows_its_reason_and_its_fix_as_an_action() {
        let mut crm = step("crm", StepStatus::Failed);
        crm.message = Some("the CRM said 401".into());
        crm.fix = Some("moneyball crm secret lz".into());
        let v = done(&crm);
        assert_eq!((v.status, v.word.as_str()), (StepState::Failed, "failed"));
        assert_eq!(v.detail, "the CRM said 401");
        assert_eq!(v.time.as_deref(), Some("1.2s"));
        assert!(matches!(v.fix, Some(Action::Terminal { .. })));
        let mut report = step("report", StepStatus::Partial);
        report.error_kind = Some("no_crm".into());
        report.counts = [("creatives".to_string(), 12)].into();
        let v = done(&report);
        assert_eq!((v.status, v.word.as_str()), (StepState::Partial, "no CRM"));
        assert_eq!(v.detail, "12 creatives");
    }

    #[test]
    fn a_run_is_always_four_steps_for_the_day_before_its_snapshot() {
        let run: RunRecord = serde_json::from_value(serde_json::json!({
            "schema": "moneyball.run/1", "workspace_id": "w", "date": "2026-10-09",
            "trigger": "manual", "started_at": "", "outcome": "failed",
            "steps": [{"name": "meta", "status": "failed", "ms": 0}],
        }))
        .unwrap();
        let v = run_view(&run);
        assert_eq!(
            (v.date.as_str(), v.day.as_str()),
            ("2026-10-08", "Thu 08 Oct")
        );
        assert_eq!(v.gaps, "stopped at step 1");
        let names: Vec<&str> = v.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, STEPS);
        assert_eq!(v.steps[3].status, StepState::Waiting);
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["outcome"], "failed");
        assert_eq!(json["steps"][0]["status"], "failed");
    }
}
