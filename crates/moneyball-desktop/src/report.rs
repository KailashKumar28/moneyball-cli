//! The daily report: which days have data, the report already written
//! for one, generating it (no network), and handing its file to the
//! browser - only ever a file inside the workspace's reports folder.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use moneyball_core::schema::CreativeReport;
use moneyball_core::{report, AppConfig};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::AppError;
use crate::run_view::data_day;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct ReportView {
    /// The snapshot the report was built from, "2026-10-09".
    pub date: String,
    /// The last day it covers, "Thu 08 Oct".
    pub day: String,
    /// Complete days it covers.
    pub days: u32,
    pub path: String,
    /// The whole page: no scripts, images inline.
    pub html: String,
    pub summary: String,
}

fn day_of(date: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

fn view(r: &CreativeReport, html_path: &Path) -> Result<ReportView, AppError> {
    let html = std::fs::read_to_string(html_path)
        .map_err(|e| AppError::new("io", format!("The report could not be read: {}", e)))?;
    let span = day_of(&r.window.since).zip(day_of(&r.window.until));
    Ok(ReportView {
        date: r.source.snapshot_date.clone(),
        day: data_day(&r.source.snapshot_date).1,
        days: span.map_or(1, |(a, b)| (b - a).num_days().max(0) as u32 + 1),
        path: html_path.display().to_string(),
        html,
        summary: report::text_summary(r),
    })
}

/// A day the Report screen can show.
#[derive(Debug, Clone, Serialize)]
pub struct ReportDay {
    /// The snapshot date, what `report_for` and `report_generate` take.
    pub date: String,
    /// The data day it reports on, "Thu 08 Oct" (the day before `date`).
    pub day: String,
    /// The window that reaches back to the 1st of the data day's month.
    pub month_to_date: u32,
}

/// The days with a snapshot, newest first.
pub fn report_days_of(cfg: &AppConfig) -> Vec<ReportDay> {
    let day = |date: String| {
        let (iso, day) = data_day(&date);
        let month_to_date = day_of(&iso).map_or(1, |d| chrono::Datelike::day(&d));
        ReportDay {
            date,
            day,
            month_to_date,
        }
    };
    snapshot_dates(cfg).into_iter().map(day).collect()
}

/// Snapshot dates, newest first.
fn snapshot_dates(cfg: &AppConfig) -> Vec<String> {
    let mut days: Vec<String> = std::fs::read_dir(cfg.snap_dir())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| day_of(name).is_some())
        .collect();
    days.sort_by(|a, b| b.cmp(a));
    days
}

/// `date` as given, checked (it names a folder), else the newest snapshot.
fn snapshot_date(cfg: &AppConfig, date: Option<String>) -> Result<Option<String>, AppError> {
    match date {
        Some(d) if day_of(&d).is_none() => Err(AppError::new(
            "config",
            format!("A report day must be YYYY-MM-DD, got \"{}\".", d),
        )),
        Some(d) => Ok(Some(d)),
        None => Ok(snapshot_dates(cfg).into_iter().next()),
    }
}

/// The daily report already written for a snapshot; None when there is
/// none (or no snapshot at all).
pub fn report_for_date(
    cfg: &AppConfig,
    date: Option<String>,
) -> Result<Option<ReportView>, AppError> {
    let Some(date) = snapshot_date(cfg, date)? else {
        return Ok(None);
    };
    match report::daily(cfg, &date) {
        Some((r, html)) => view(&r, &html).map(Some),
        None => Ok(None),
    }
}

pub fn generate(cfg: &AppConfig, date: Option<String>, days: u32) -> Result<ReportView, AppError> {
    let date = snapshot_date(cfg, date)?;
    let out = report::generate(cfg, date.as_deref(), days.max(1))?;
    view(&out.report, &out.html_path)
}

/// `path` resolved, if it is an HTML file inside this workspace's
/// reports folder. Symlinks and `..` are resolved before the check.
pub fn report_file(cfg: &AppConfig, path: &str) -> Result<PathBuf, AppError> {
    let refuse = || AppError::new("refused", "Only a report in this workspace can be opened.");
    let reports = cfg.mb_dir().join("reports").canonicalize();
    let file = Path::new(path).canonicalize();
    let (Ok(reports), Ok(file)) = (reports, file) else {
        return Err(refuse());
    };
    let html = file.extension().is_some_and(|e| e == "html");
    if file.starts_with(&reports) && file.is_file() && html {
        Ok(file)
    } else {
        Err(refuse())
    }
}

pub fn refresh_busy() -> AppError {
    AppError::busy(
        "A refresh is running and writes the report itself. Generate one when it finishes.",
    )
}

#[tauri::command(async)]
pub fn report_days(state: State<'_, AppState>) -> Result<Vec<ReportDay>, AppError> {
    Ok(report_days_of(&state.workspace()?))
}

#[tauri::command(async)]
pub fn report_for(
    state: State<'_, AppState>,
    date: Option<String>,
) -> Result<Option<ReportView>, AppError> {
    report_for_date(&state.workspace()?, date)
}

#[tauri::command(async)]
pub fn report_generate(
    state: State<'_, AppState>,
    date: Option<String>,
    days: u32,
) -> Result<ReportView, AppError> {
    // A refresh writes the same file as its last step.
    if state.refresh_running() {
        return Err(refresh_busy());
    }
    generate(&state.workspace()?, date, days)
}

/// The opener runs here, in Rust: the webview has no permission to open
/// anything by itself.
#[tauri::command(async)]
pub fn report_open_in_browser(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), AppError> {
    let file = report_file(&state.workspace()?, &path)?;
    app.opener()
        .open_path(file.display().to_string(), None::<&str>)
        .map_err(|e| AppError::new("io", format!("The report could not be opened: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_html_file_under_the_workspace_reports_folder_opens() {
        let root = std::env::temp_dir().join(format!("mb-desktop-report-{}", std::process::id()));
        let reports = root.join(".moneyball/reports/2026-10-09");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("Acme-Daily-08-Oct-2026.html"), "<p>").unwrap();
        std::fs::write(reports.join("creative-report.json"), "{}").unwrap();
        std::fs::write(root.join("outside.html"), "<p>").unwrap();
        let cfg = AppConfig::resolve_optional(root.to_str(), None);
        let at = |p: &str| reports.join(p).display().to_string();
        assert!(report_file(&cfg, &at("Acme-Daily-08-Oct-2026.html")).is_ok());
        for bad in [
            at("creative-report.json"),
            at("missing.html"),
            at("../../../outside.html"),
            root.join("outside.html").display().to_string(),
            "/etc/hosts".to_string(),
            String::new(),
        ] {
            let err = report_file(&cfg, &bad).unwrap_err();
            assert_eq!(err.kind, "refused", "{}", bad);
        }
        assert!(snapshot_date(&cfg, Some("../x".into())).is_err());
        std::fs::remove_dir_all(&root).ok();
    }
}
