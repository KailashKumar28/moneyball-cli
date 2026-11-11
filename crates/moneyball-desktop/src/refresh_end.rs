//! A finished refresh in the interface's words: the run, core's verdict
//! under the steps, the fixes and the rerun as actions, and the day's
//! numbers from the report the run wrote.

use moneyball_core::landing::day_numbers;
use moneyball_core::refresh;
use moneyball_core::schema::{CreativeReport, RunRecord};
use moneyball_core::words::Tint;
use serde::Serialize;

use crate::action::{translate, Action};
use crate::run_view::{run_view, RunView};
use crate::today::{numbers, NumberView};

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

/// `last_good`: the data day on screen before the run; `day`: the day
/// the run fills.
pub fn finished(run: &RunRecord, ms: u64, last_good: Option<&str>, day: &str) -> Finished {
    let fixes: Vec<Action> = refresh::fixes(run).iter().map(translate).collect();
    // The run says where its report went; the numbers come from its json.
    let report = run
        .artifacts
        .get("report_json")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<CreativeReport>(&raw).ok());
    let (tiles, note) = report.map(|r| numbers(&day_numbers(&r))).unzip();
    Finished {
        run: run_view(run),
        time: refresh::duration(ms),
        verdict: refresh::verdict(run, last_good, day).map(|v| VerdictView {
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
    }
}
