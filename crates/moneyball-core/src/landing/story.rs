//! What is wrong with the data, since when, the fixes the run records
//! name, and the one command that brings the data in.

use chrono::{Duration, NaiveDate};

use super::{Headline, Landing, DAY};
use crate::health::{Health, State};
use crate::refresh::{self, REUSES_META};
use crate::schema::{RunRecord, RunStep, StepStatus, Trigger};
use crate::words::{Command, Tint};

const NOT_CONNECTED: &str =
    "No CRM is connected, so CRM leads, qualified and every verdict are off.";

/// The headline, its details and the fixes for this state.
pub(super) fn tell(h: &Health, day: &str, scheduled: bool) -> Landing {
    let mut out = Landing::default();
    let (text, rest) = match h.state {
        State::Fresh => return out,
        State::NoData => (
            "No data yet.".to_string(),
            "Nothing has been pulled for this workspace.".to_string(),
        ),
        State::Stale => stale(h, scheduled, &mut out.details),
        State::Failed => failed(h, day, &mut out),
        State::CrmMissing => crm_missing(h, day, &mut out.fixes),
        State::Partial { have, of } => (
            format!("{} of {} products have data for {}.", have, of, day),
            step(h.run.as_ref(), "meta")
                .and_then(|s| s.message.clone())
                .unwrap_or_default(),
        ),
        State::Pinned => pinned(h, day),
    };
    let tint = match h.state {
        State::Failed => Tint::Bad,
        // A chosen day is not a fault.
        State::Pinned => Tint::Plain,
        _ => Tint::Warn,
    };
    out.headline = Some(Headline { tint, text, rest });
    out
}

fn step<'a>(run: Option<&'a RunRecord>, name: &str) -> Option<&'a RunStep> {
    run.and_then(|r| r.steps.iter().find(|s| s.name == name))
}

fn stale(h: &Health, scheduled: bool, details: &mut Vec<String>) -> (String, String) {
    let mut rest = Vec::new();
    if let Some(t) = h.refreshed_at {
        rest.push(format!(
            "Last refresh: {}{}.",
            t.format("%a %d %b, %-I:%M %P"),
            h.run
                .as_ref()
                .map_or(String::new(), |r| format!(" ({})", how(r)))
        ));
    }
    rest.extend(h.snap_date.and_then(|snap| missed(snap, h.today)));
    details.extend(crm_behind(h));
    if !scheduled {
        details.push("Nothing refreshes on its own yet.".into());
    }
    (
        format!("Your data is {} days old.", h.age_days().unwrap_or(0)),
        rest.join(" "),
    )
}

fn failed(h: &Health, day: &str, out: &mut Landing) -> (String, String) {
    let run = h.failed_today.as_ref();
    let step = run.and_then(|f| f.steps.iter().find(|s| s.status == StepStatus::Failed));
    let rest = step.map_or(String::new(), |s| {
        format!(
            "{}: {}",
            refresh::step_label(&s.name),
            s.message.clone().unwrap_or_default()
        )
    });
    // A first refresh that failed has no earlier data to speak of.
    if h.snap_date.is_some() {
        out.details
            .push(format!("Your last good data ({}) is still here.", day));
    }
    out.fixes = run.map(refresh::fixes).unwrap_or_default();
    ("Today's refresh failed.".into(), rest)
}

fn crm_missing(h: &Health, day: &str, fixes: &mut Vec<Command>) -> (String, String) {
    let rest = match step(h.run.as_ref(), "crm") {
        _ if !h.crm_connected => NOT_CONNECTED.to_string(),
        Some(s) if s.status == StepStatus::Failed => format!(
            "{}.",
            s.message
                .as_deref()
                .unwrap_or_default()
                .trim_end_matches('.')
        ),
        _ => "CRM was not pulled into this snapshot, so every verdict is withheld.".to_string(),
    };
    *fixes = h.run.as_ref().map(refresh::fixes).unwrap_or_default();
    if !h.crm_connected && fixes.is_empty() {
        fixes.push(Command::fix("/connect crm"));
    }
    // A CRM that stopped days ago is not one day's gap: say since when.
    let headline = match (h.crm_day(), h.crm_snap_date) {
        (Some(last), Some(c)) if h.crm_connected && Some(c) < h.snap_date => {
            format!("No CRM data since {}.", last.format(DAY))
        }
        _ => format!("{} has no CRM data.", day),
    };
    (headline, rest)
}

fn pinned(h: &Health, day: &str) -> (String, String) {
    let newest = h
        .newest_snap_date
        .filter(|n| Some(*n) != h.snap_date)
        .and_then(|n| n.pred_opt())
        .map(|n| format!("The newest data is {}.", n.format(DAY)))
        .unwrap_or_default();
    (format!("Viewing {} (--date).", day), newest)
}

/// The one command that brings the data in; None when nothing is asked.
pub(super) fn next_command(h: &Health) -> Option<Command> {
    let (cmd, does) = match h.state {
        State::Fresh | State::Pinned => return None,
        State::Partial { .. } => ("/refresh --force", "pulls Meta again for every product"),
        State::CrmMissing if h.age_days() == Some(0) => ("/refresh", REUSES_META),
        _ => (
            "/refresh",
            "pulls Meta, then CRM, then writes yesterday's report (about 2 minutes)",
        ),
    };
    Some(Command::new(cmd, does))
}

pub(super) fn how(run: &RunRecord) -> &'static str {
    match run.trigger {
        Trigger::Schedule => "scheduled",
        Trigger::Manual => "manual",
    }
}

/// "9 mornings missed, 29 Sep to 07 Oct." - the days between the
/// snapshot and today that never got a refresh.
pub(super) fn missed(snap: NaiveDate, today: NaiveDate) -> Option<String> {
    let (from, to) = (snap + Duration::days(1), today - Duration::days(1));
    match (to - from).num_days() + 1 {
        n if n < 1 => None,
        1 => Some(format!("1 morning missed, {}.", from.format("%d %b"))),
        n => Some(format!(
            "{} mornings missed, {} to {}.",
            n,
            from.format("%d %b"),
            to.format("%d %b")
        )),
    }
}

/// Why CRM is behind the Meta data, when it is.
fn crm_behind(h: &Health) -> Option<String> {
    const OFF: &str = "so CRM leads, qualified and every verdict are off until a refresh.";
    if !h.crm_connected {
        return Some(NOT_CONNECTED.into());
    }
    match h.crm_snap_date {
        None => Some(format!("CRM has never been pulled, {}", OFF)),
        Some(c) if Some(c) < h.snap_date => Some(format!(
            "CRM is older still: last pulled {}, {}",
            c.format("%d %b"),
            OFF
        )),
        _ => None,
    }
}
