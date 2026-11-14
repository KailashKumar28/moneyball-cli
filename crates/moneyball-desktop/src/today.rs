//! Today: how fresh the data is, what to do about it, the last run, and
//! the day's daily report - the page a client gets - even when the data
//! is stale (its day says how old). Core's landing with every command
//! translated. Recomputed on each call (it reads the clock).

use moneyball_core::brief::{self, Loaded};
use moneyball_core::health::Health;
use moneyball_core::landing::{self, Headline};
use moneyball_core::words::Command;
use moneyball_core::{schedule, AppConfig};
use serde::Serialize;
use tauri::State;

use crate::action::{translate, Action};
use crate::error::AppError;
use crate::report::{report_for_date, ReportView};
use crate::run_view::{run_view, RunView, DAY};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct Today {
    pub health: Health,
    /// The footer's word: "Fresh", "CRM missing".
    pub health_label: String,
    /// The footer's two lines: the data day, the CRM day.
    pub footer: [String; 2],
    pub headline: Option<Headline>,
    pub details: Vec<String>,
    pub fixes: Vec<Action>,
    pub next: Option<Action>,
    /// The data day on screen; None with no data.
    pub day: Option<DayView>,
    /// The run behind the data on screen (today's, when it failed).
    pub last_run: Option<RunView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportMissing {
    pub note: String,
    pub next: Action,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayView {
    /// "Thu 08 Oct".
    pub day: String,
    /// "07:31 this morning (manual, 1m 21s)"; None when unknown.
    pub refreshed: Option<String>,
    /// The day's daily report, as the Report screen shows it; None when
    /// there is none, and `report_missing` says so.
    pub report: Option<ReportView>,
    pub report_missing: Option<ReportMissing>,
}

/// The data day on screen and its report, whatever the health: a stale
/// day's report is still the newest there is, and the day and the
/// headline above it say how old.
fn day_view(cfg: &AppConfig, h: &Health) -> Option<DayView> {
    let day = h.data_day()?.format(DAY).to_string();
    let date = h.snap_date?.format("%Y-%m-%d").to_string();
    let missing = |note: String| ReportMissing {
        note,
        next: translate(&Command::new(
            "/report",
            "builds it from this day's snapshot, no network",
        )),
    };
    let (report, report_missing) = match report_for_date(cfg, Some(date)) {
        Ok(Some(r)) => (Some(r), None),
        Ok(None) => (
            None,
            Some(missing(
                "No daily report is written for this day yet.".into(),
            )),
        ),
        Err(e) => (None, Some(missing(e.reason))),
    };
    Some(DayView {
        day,
        refreshed: landing::refreshed(h),
        report,
        report_missing,
    })
}

/// The footer's two lines: the data day and the CRM day. How old the
/// data is lives in the state above and on Today and Brief; an age here
/// would not fit the line.
fn footer(h: &Health) -> [String; 2] {
    let Some(day) = h.data_day() else {
        return ["No data yet".into(), String::new()];
    };
    let data = format!("Data {}", day.format(DAY));
    let crm = match (h.crm_connected, h.crm_day()) {
        (_, Some(d)) => format!("CRM {}", d.format("%d %b")),
        (false, None) => "CRM not connected".into(),
        (true, None) => "CRM never pulled".into(),
    };
    [data, crm]
}

pub(crate) fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Today for one loading of the workspace. `scheduled`: a morning
/// refresh is installed (the landing then stops saying none is).
pub fn today_view(cfg: &AppConfig, loaded: &Loaded, scheduled: bool) -> Today {
    let h = &loaded.health;
    let land = landing::landing(cfg, h, scheduled);
    // Today's failed run is the news; else the run behind the data shown.
    let last_run = h.failed_today.as_ref().or(h.run.as_ref());
    Today {
        health_label: capitalized(&h.label()),
        footer: footer(h),
        headline: land.headline,
        details: land.details,
        fixes: land.fixes.iter().map(translate).collect(),
        next: land.next.as_ref().map(translate),
        day: day_view(cfg, h),
        last_run: last_run.map(run_view),
        health: h.clone(),
    }
}

#[tauri::command(async)]
pub fn today(state: State<'_, AppState>) -> Result<Today, AppError> {
    let cfg = state.workspace()?;
    let scheduled = schedule::installed(&cfg).is_some();
    Ok(today_view(&cfg, &brief::load(&cfg), scheduled))
}
