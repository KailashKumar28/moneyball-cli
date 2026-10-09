//! `/status` and `moneyball status` (frame R6): the last data days, one
//! row each - what landed for it (Meta, CRM, leads, report) and how its
//! run went - with missed days as rows, never gaps. File reads only.

use chrono::{DateTime, Duration, Local, NaiveDate, Timelike};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::refresh::{duration, status_word, step_label};
use crate::schema::{Outcome, RunRecord, StepStatus, Trigger};

/// Column widths of the table, shared by the shell and the TUI.
pub const WIDTHS: [usize; 5] = [13, 7, 7, 7, 10];
pub const HEADER: [&str; 6] = ["data for", "Meta", "CRM", "leads", "report", "ran"];

/// One column's word for a day.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Ok,
    /// The day has data, but not this part of it.
    None,
    /// In, with a gap: "4 of 5", "no CRM".
    Warn(String),
    /// Nothing for the day at all.
    Absent,
}

impl Mark {
    pub fn word(&self) -> &str {
        match self {
            Mark::Ok => "ok",
            Mark::None => "none",
            Mark::Warn(w) => w,
            Mark::Absent => "--",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Normal,
    Missed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Day {
    /// The data day (YYYY-MM-DD); its snapshot is the day after.
    pub date: NaiveDate,
    pub meta: Mark,
    pub crm: Mark,
    pub leads: Mark,
    pub report: Mark,
    /// How its run went, in words: "Fri 07:30 scheduled, 1m 18s".
    pub ran: String,
    pub tone: Tone,
    #[serde(skip)]
    pub run: Option<RunRecord>,
}

/// The last `n` data days, newest first: yesterday, then back.
pub fn days(cfg: &AppConfig, n: usize) -> Vec<Day> {
    let today = Local::now().date_naive();
    let since = crate::schedule::since(cfg);
    (1..=n as i64)
        .map(|back| day(cfg, today - Duration::days(back), since))
        .collect()
}

fn day(cfg: &AppConfig, date: NaiveDate, scheduled_since: Option<NaiveDate>) -> Day {
    let snap = (date + Duration::days(1)).format("%Y-%m-%d").to_string();
    let dir = cfg.snap_dir().join(&snap);
    let ads = dir.join("ads_daily.json");
    let has = ads.is_file();
    let run = crate::runs::read(cfg, &snap);
    let step = |name: &str| {
        run.as_ref()
            .and_then(|r| r.steps.iter().find(|s| s.name == name))
    };
    let mark = |present: bool| match (has, present) {
        (false, _) => Mark::Absent,
        (true, true) => Mark::Ok,
        (true, false) => Mark::None,
    };
    let meta = match step("meta") {
        Some(s) if has && s.status == StepStatus::Partial => Mark::Warn(status_word(s)),
        _ => mark(true),
    };
    let crm_json = std::fs::metadata(dir.join("crm.json")).is_ok_and(|m| m.len() > 2);
    let report = match report_has_crm(cfg, &snap) {
        Some(true) => Mark::Ok,
        Some(false) => Mark::Warn("no CRM".into()),
        None => mark(false),
    };
    let pulled = || {
        let t: DateTime<Local> = std::fs::metadata(&ads).ok()?.modified().ok()?.into();
        Some(t)
    };
    let (ran, tone) = match &run {
        Some(r) => ran(r, cfg),
        None if has => (
            pulled().map_or("before run records".into(), |t| {
                format!("{}, before run records", t.format("%a %H:%M"))
            }),
            Tone::Normal,
        ),
        // Its run would have been the morning after the data day.
        None => match scheduled_since {
            Some(s) if s <= date + Duration::days(1) => (
                "missed: the scheduled run never started".into(),
                Tone::Missed,
            ),
            _ => ("missed: nothing scheduled yet".into(), Tone::Missed),
        },
    };
    Day {
        date,
        meta,
        crm: mark(crm_json),
        leads: mark(dir.join("leads.json").is_file()),
        report,
        ran,
        tone,
        run,
    }
}

/// "Fri 07:30 scheduled, 1m 18s", and what stopped it when it failed.
fn ran(r: &RunRecord, cfg: &AppConfig) -> (String, Tone) {
    let started = local(&r.started_at);
    let at = started.map_or(String::new(), |t| format!("{} ", t.format("%a %H:%M")));
    let how = if r.trigger == Trigger::Schedule {
        "scheduled"
    } else {
        "manual"
    };
    let took = match (started, local(&r.finished_at)) {
        (Some(a), Some(b)) => duration((b - a).num_milliseconds().max(0) as u64),
        _ => String::new(),
    };
    match r.outcome {
        Outcome::Running if crate::runs::lock(cfg).is_ok() => (
            format!("{}{}: stopped before it finished", at, how),
            Tone::Failed,
        ),
        Outcome::Running => (format!("{}{}: running now", at, how), Tone::Normal),
        Outcome::Failed => {
            let what = r
                .steps
                .iter()
                .find(|s| s.status == StepStatus::Failed)
                .map_or("refresh".into(), |s| step_label(&s.name).to_string());
            (format!("{}{}: {} failed", at, how, what), Tone::Failed)
        }
        _ if took.is_empty() => (format!("{}{}", at, how), Tone::Normal),
        _ => (format!("{}{}, {}", at, how, took), Tone::Normal),
    }
}

/// Whether the snapshot's daily report carries CRM; None without one.
fn report_has_crm(cfg: &AppConfig, snap: &str) -> Option<bool> {
    #[derive(Deserialize)]
    struct Head {
        source: Source,
    }
    #[derive(Deserialize)]
    struct Source {
        crm_present: bool,
    }
    let path = cfg
        .mb_dir()
        .join("reports")
        .join(snap)
        .join("creative-report.json");
    let head: Head = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(head.source.crm_present)
}

fn local(stamp: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|t| t.with_timezone(&Local))
}

/// "07:30 this morning", "16:05 today", "yesterday 23:41", "Mon 28 Sep 11:41".
pub fn when(t: DateTime<Local>, today: NaiveDate) -> String {
    let hm = t.format("%H:%M");
    match (today - t.date_naive()).num_days() {
        0 if t.hour() < 12 => format!("{} this morning", hm),
        0 => format!("{} today", hm),
        1 => format!("yesterday {}", hm),
        _ => t.format("%a %d %b %H:%M").to_string(),
    }
}

/// A table row, or a run of missed days folded into one line.
pub enum Row<'a> {
    Day(&'a Day),
    Missed {
        count: usize,
        from: NaiveDate,
        to: NaiveDate,
    },
}

/// Missed days stay rows, but a long run of them shows its first two
/// and folds the rest ("... 7 more missed days, 28 Sep to 04 Oct").
pub fn rows(days: &[Day]) -> Vec<Row<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < days.len() {
        let run = days[i..]
            .iter()
            .take_while(|d| d.tone == Tone::Missed)
            .count();
        if run > 3 {
            out.extend(days[i..i + 2].iter().map(Row::Day));
            out.push(Row::Missed {
                count: run - 2,
                from: days[i + run - 1].date,
                to: days[i + 2].date,
            });
            i += run;
        } else {
            out.push(Row::Day(&days[i]));
            i += 1;
        }
    }
    out
}

/// The footer: how Meta and the CRM last answered, and where records live.
pub fn footer(cfg: &AppConfig, days: &[Day]) -> Vec<(&'static str, String)> {
    let today = Local::now().date_naive();
    let last = |name: &str| {
        days.iter().find_map(|d| {
            let r = d.run.as_ref()?;
            let s = r
                .steps
                .iter()
                .find(|s| s.name == name && s.status != StepStatus::Skipped)?;
            // A reused pull asked Meta nothing new: date it by the data it
            // reused, written when Meta last answered.
            let at = if s.message.as_deref() == Some(crate::refresh::REUSED) {
                let ads = cfg.snap_dir().join(&r.date).join("ads_daily.json");
                Some(DateTime::<Local>::from(
                    std::fs::metadata(ads).ok()?.modified().ok()?,
                ))
            } else {
                local(&r.finished_at)
            };
            Some((s.clone(), at))
        })
    };
    let pull = |label: &str, name: &str| match last(name) {
        Some((s, t)) => {
            let at = t.map_or(String::new(), |t| format!(" {}", when(t, today)));
            match (&s.status, &s.fix) {
                (StepStatus::Failed, Some(fix)) => format!("{} failed{} - fix: {}", label, at, fix),
                (StepStatus::Failed, None) => format!("{} failed{}", label, at),
                _ => format!("{} ok{}", label, at),
            }
        }
        None => format!("{}: no pull on record", label),
    };
    let crm = if crate::crm::fetch::spec_path(cfg).is_file() {
        pull("CRM", "crm")
    } else {
        "CRM not connected".into()
    };
    let schedule = match crate::schedule::installed(cfg) {
        Some(at) => {
            let next = crate::schedule::next_from_now(at);
            format!(
                "every day {:02}:{:02}  \u{00B7}  next {}",
                at.0,
                at.1,
                next.format("%a %d %b %H:%M")
            )
        }
        None => "none: a refresh runs when you ask for one".to_string(),
    };
    vec![
        ("schedule", schedule),
        (
            "access",
            format!("{}  \u{00B7}  {}", pull("Meta", "meta"), crm),
        ),
        ("records", cfg.runs_dir().display().to_string()),
    ]
}

/// The table as plain lines, for the shell.
pub fn lines(cfg: &AppConfig, days: &[Day]) -> Vec<String> {
    let cell = |w: usize, s: &str| format!("{:<w$}", s);
    let mut out = vec![format!(
        "{}{}{}{}{}{}",
        cell(WIDTHS[0], HEADER[0]),
        cell(WIDTHS[1], HEADER[1]),
        cell(WIDTHS[2], HEADER[2]),
        cell(WIDTHS[3], HEADER[3]),
        cell(WIDTHS[4], HEADER[4]),
        HEADER[5]
    )];
    for row in rows(days) {
        out.push(match row {
            Row::Day(d) => format!(
                "{}{}{}{}{}{}",
                cell(WIDTHS[0], &d.date.format("%a %d %b").to_string()),
                cell(WIDTHS[1], d.meta.word()),
                cell(WIDTHS[2], d.crm.word()),
                cell(WIDTHS[3], d.leads.word()),
                cell(WIDTHS[4], d.report.word()),
                d.ran
            ),
            Row::Missed { count, from, to } => missed_line(count, from, to),
        });
    }
    out.push(String::new());
    for (k, v) in footer(cfg, days) {
        out.push(format!("{:<11}{}", k, v));
    }
    out
}

pub fn missed_line(count: usize, from: NaiveDate, to: NaiveDate) -> String {
    format!(
        "... {} more missed days, {} to {}",
        count,
        from.format("%d %b"),
        to.format("%d %b")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: &str, tone: Tone) -> Day {
        Day {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            meta: Mark::Absent,
            crm: Mark::Absent,
            leads: Mark::Absent,
            report: Mark::Absent,
            ran: String::new(),
            tone,
            run: None,
        }
    }

    #[test]
    fn long_runs_of_missed_days_fold_short_ones_stay() {
        let mut days = vec![day("2026-10-08", Tone::Normal)];
        for d in (1..=7).rev() {
            days.push(day(&format!("2026-10-0{}", d), Tone::Missed));
        }
        days.push(day("2026-09-30", Tone::Normal));
        days.push(day("2026-09-29", Tone::Missed));
        days.push(day("2026-09-28", Tone::Missed));
        let rows = rows(&days);
        assert_eq!(rows.len(), 1 + 2 + 1 + 1 + 2);
        match rows[3] {
            Row::Missed { count, from, to } => {
                assert_eq!(count, 5);
                assert_eq!(
                    missed_line(count, from, to),
                    "... 5 more missed days, 01 Oct to 05 Oct"
                );
            }
            _ => panic!("the fourth row folds the run"),
        }
    }
}
