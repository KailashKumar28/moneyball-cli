//! A finished refresh in the interface's words: the run, core's verdict
//! under the steps, the fixes and the rerun as actions, and the day's
//! numbers from the report the run wrote. Built from the run record
//! alone, so the live event and a later reading of the record agree.

use chrono::DateTime;
use moneyball_core::landing::{self, day_numbers};
use moneyball_core::refresh;
use moneyball_core::schema::{CreativeReport, RunRecord};
use moneyball_core::words::Tint;
use serde::Serialize;

use crate::action::{translate, Action};
use crate::run_view::{run_view, RunView};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NumberView {
    pub label: String,
    pub value: String,
    /// Where the number comes from; None for one this app does not know.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Meta,
    Crm,
}

/// Which system a day number (landing/day.rs `day_numbers`) is read from.
fn source(label: &str) -> Option<Source> {
    match label {
        "spend" | "Meta leads" => Some(Source::Meta),
        "CRM leads" | "qualified" | "cost per qualified" => Some(Source::Crm),
        _ => None,
    }
}

/// The day's numbers, and the note said in place of the CRM ones.
fn numbers(n: &landing::DayNumbers) -> (Vec<NumberView>, Option<String>) {
    let tile = |n: &landing::Number| NumberView {
        source: source(&n.label),
        label: n.label.clone(),
        value: n.value.clone(),
    };
    (n.numbers.iter().map(tile).collect(), n.note.clone())
}

#[derive(Debug, Clone, Serialize)]
pub struct VerdictView {
    pub tint: Tint,
    pub headline: String,
    pub rest: String,
    pub next: Option<Action>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finished {
    pub run: RunView,
    /// How long the whole run took: "1m 21s".
    pub time: String,
    /// The sentence under the steps; None for a clean run.
    pub verdict: Option<VerdictView>,
    pub fixes: Vec<Action>,
    /// The refresh to run once the fixes are done; None with no fixes.
    pub rerun: Option<Action>,
    /// None when the run wrote no report.
    pub numbers: Option<Vec<NumberView>>,
    pub numbers_note: Option<String>,
    pub report_path: Option<String>,
}

/// How long a recorded run took, from the times it wrote; the sum of
/// its steps when those will not parse.
pub fn recorded_ms(run: &RunRecord) -> u64 {
    let at = |t: &str| DateTime::parse_from_rfc3339(t).ok();
    match at(&run.started_at).zip(at(&run.finished_at)) {
        Some((start, end)) => (end - start).num_milliseconds().max(0) as u64,
        None => run.steps.iter().map(|s| s.ms).sum(),
    }
}

/// `last_good`: the data day of the newest good snapshot, said when the
/// run failed.
pub fn finished(run: &RunRecord, ms: u64, last_good: Option<&str>) -> Finished {
    let view = run_view(run);
    let fixes: Vec<Action> = refresh::fixes(run).iter().map(translate).collect();
    // The run says where its report went; the numbers come from its json.
    let report = run
        .artifacts
        .get("report_json")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<CreativeReport>(&raw).ok());
    let (tiles, note) = report.map(|r| numbers(&day_numbers(&r))).unzip();
    Finished {
        time: refresh::duration(ms),
        // The day the run fills is its record's data day.
        verdict: refresh::verdict(run, last_good, &view.day).map(|v| VerdictView {
            tint: v.tint,
            headline: v.headline,
            rest: v.rest,
            next: v.next.as_ref().map(translate),
        }),
        rerun: (!fixes.is_empty()).then(|| translate(&refresh::rerun(run))),
        fixes,
        numbers: tiles,
        numbers_note: note.flatten(),
        report_path: run.artifacts.get("report").cloned(),
        run: view,
    }
}
