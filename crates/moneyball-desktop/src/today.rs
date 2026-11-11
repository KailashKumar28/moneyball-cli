//! Today: how fresh the data is, what to do about it, the day's numbers
//! and the last run. Core's landing with every command translated and
//! every path left out. Recomputed on each call (it reads the clock).

use moneyball_core::brief::{self, Loaded};
use moneyball_core::health::{Health, State as Fresh, STALE_DAYS};
use moneyball_core::landing::{self, DayBlock, DayReport, Headline};
use moneyball_core::words::Tint;
use moneyball_core::{schedule, AppConfig};
use serde::Serialize;
use tauri::State;

use crate::action::{translate, Action};
use crate::error::AppError;
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
    pub day: Option<DayView>,
    /// The run behind the data-health table.
    pub last_run: Option<RunView>,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeView {
    pub label: String,
    pub text: String,
    pub tint: Tint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersusView {
    pub day: String,
    pub changes: Vec<ChangeView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportMissing {
    pub note: String,
    pub next: Action,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayView {
    pub day: String,
    pub refreshed: Option<String>,
    /// None = no report for the day.
    pub numbers: Option<Vec<NumberView>>,
    pub numbers_note: Option<String>,
    pub versus: Option<VersusView>,
    pub report_missing: Option<ReportMissing>,
}

/// The day's numbers as tiles, and the note said in place of the CRM ones.
pub fn numbers(n: &landing::DayNumbers) -> (Vec<NumberView>, Option<String>) {
    let tile = |n: &landing::Number| NumberView {
        source: source(&n.label),
        label: n.label.clone(),
        value: n.value.clone(),
    };
    (n.numbers.iter().map(tile).collect(), n.note.clone())
}

fn day_view(block: DayBlock) -> DayView {
    let mut out = DayView {
        day: block.day,
        refreshed: block.refreshed,
        numbers: None,
        numbers_note: None,
        versus: None,
        report_missing: None,
    };
    match block.report {
        DayReport::Missing { note, next } => {
            out.report_missing = Some(ReportMissing {
                note,
                next: translate(&next),
            })
        }
        DayReport::Written {
            numbers: n, versus, ..
        } => {
            let (tiles, note) = numbers(&n);
            out.numbers = Some(tiles);
            out.numbers_note = note;
            out.versus = versus.map(|v| VersusView {
                day: v.day,
                changes: v.changes.into_iter().map(change).collect(),
            });
        }
    }
    out
}

fn change(c: landing::Change) -> ChangeView {
    ChangeView {
        label: c.label,
        text: c.text,
        tint: c.tint,
    }
}

/// The footer's two lines, in the words of the TUI's context bar.
fn footer(h: &Health) -> [String; 2] {
    let (Some(day), Some(age)) = (h.data_day(), h.age_days()) else {
        return ["No data yet".into(), String::new()];
    };
    let data = if age >= STALE_DAYS && h.state != Fresh::Pinned {
        format!("Data {}, pulled {} days ago", day.format("%d %b"), age)
    } else {
        format!("Data {}", day.format(DAY))
    };
    let crm = match (h.crm_connected, h.crm_day()) {
        (_, Some(d)) => format!("CRM {}", d.format("%d %b")),
        (false, None) => "CRM not connected".into(),
        (true, None) => "CRM never pulled".into(),
    };
    [data, crm]
}

fn capitalized(s: &str) -> String {
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
        day: land.day.map(day_view),
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
