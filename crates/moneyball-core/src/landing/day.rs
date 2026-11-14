//! The day on screen: when and how it was refreshed, its numbers, and
//! the change against the last comparable day.

use chrono::{DateTime, NaiveDate};

use super::story::how;
use super::{Change, DayBlock, DayNumbers, DayReport, Number, Versus, DAY};
use crate::config::AppConfig;
use crate::health::Health;
use crate::refresh;
use crate::report::daily;
use crate::schema::{CreativeReport, RunRecord};
use crate::text::commas;
use crate::words::{Command, Tint};

/// When and how the data on screen was refreshed: "07:30 this morning
/// (scheduled, 1m 21s)"; None when unknown.
pub fn refreshed(h: &Health) -> Option<String> {
    let mut when = h
        .refreshed_at
        .map(|t| crate::status::when(t, h.today))
        .unwrap_or_default();
    if let Some(run) = &h.run {
        let ms = elapsed_ms(run);
        let took = if ms > 0 {
            format!(", {}", refresh::duration(ms))
        } else {
            String::new()
        };
        when.push_str(&format!(" ({}{})", how(run), took));
    }
    Some(when).filter(|w| !w.is_empty())
}

pub(super) fn day_block(cfg: &AppConfig, h: &Health, day: String) -> DayBlock {
    let found = h
        .snap_date
        .and_then(|s| daily(cfg, &s.format("%Y-%m-%d").to_string()));
    let report = match found {
        None => DayReport::Missing {
            note: "no daily report for this day yet".into(),
            next: Command::new("/refresh", "writes one"),
        },
        Some((rep, path)) => DayReport::Written {
            numbers: day_numbers(&rep),
            versus: prior(cfg, &rep).map(|before| versus(&rep, &before)),
            path,
        },
    };
    DayBlock {
        day,
        refreshed: refreshed(h),
        report,
    }
}

/// The report before this one, when it is within the last week: an older
/// one says little.
fn prior(cfg: &AppConfig, rep: &CreativeReport) -> Option<CreativeReport> {
    let day_of = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    let before = rep.prior_date.as_deref()?;
    let recent = (day_of(&rep.report_date)? - day_of(before)?).num_days() <= 7;
    recent.then(|| daily(cfg, before)).flatten().map(|(r, _)| r)
}

/// The day's numbers as the report has them: spend, Meta leads, then CRM
/// leads, qualified and cost per qualified when it has CRM.
pub fn day_numbers(report: &CreativeReport) -> DayNumbers {
    let k = &report.portfolio;
    let number = |label: &str, value: String| Number {
        phrase: format!("{} {}", label, value),
        label: label.into(),
        value,
    };
    let mut numbers = vec![
        number("spend", format!("Rs.{}", commas(k.spend.round() as u64))),
        number("Meta leads", k.funnel.m_leads.to_string()),
    ];
    if !report.source.crm_present {
        return DayNumbers {
            numbers,
            note: Some("no CRM in this report".into()),
        };
    }
    numbers.push(number("CRM leads", k.funnel.l_leads.to_string()));
    numbers.push(number("qualified", k.funnel.qualified.to_string()));
    if let Some(c) = k.cost_per_qualified {
        let value = format!("Rs.{}", commas(c.round() as u64));
        numbers.push(Number {
            label: "cost per qualified".into(),
            phrase: format!("{} per qualified", value),
            value,
        });
    }
    DayNumbers {
        numbers,
        note: None,
    }
}

/// Qualified and cost per qualified against `before`, or spend and Meta
/// leads when either day has no CRM.
fn versus(now: &CreativeReport, before: &CreativeReport) -> Versus {
    let day = NaiveDate::parse_from_str(&before.window.until, "%Y-%m-%d")
        .map(|d| d.format(DAY).to_string())
        .unwrap_or_else(|_| before.window.until.clone());
    let (a, b) = (&now.portfolio, &before.portfolio);
    let change = |label: &str, by: i64, unit: &str, tint: Tint| Change {
        label: label.into(),
        by,
        text: signed(by, unit),
        tint,
    };
    let good = |better: bool, by: i64| match by {
        0 => Tint::Plain,
        _ if better => Tint::Good,
        _ => Tint::Bad,
    };
    let mut changes = Vec::new();
    if now.source.crm_present && before.source.crm_present {
        let dq = a.funnel.qualified as i64 - b.funnel.qualified as i64;
        changes.push(change("qualified", dq, "", good(dq > 0, dq)));
        if let (Some(x), Some(y)) = (a.cost_per_qualified, b.cost_per_qualified) {
            let dc = x.round() as i64 - y.round() as i64;
            changes.push(change("cost per qualified", dc, "Rs.", good(dc < 0, dc)));
        }
    } else {
        let ds = a.spend.round() as i64 - b.spend.round() as i64;
        let dl = a.funnel.m_leads as i64 - b.funnel.m_leads as i64;
        // More spend is neither good nor bad on its own.
        changes.push(change("spend", ds, "Rs.", Tint::Plain));
        changes.push(change("Meta leads", dl, "", good(dl > 0, dl)));
    }
    Versus { day, changes }
}

pub(super) fn signed(n: i64, unit: &str) -> String {
    let sign = if n < 0 { "-" } else { "+" };
    format!("{}{}{}", sign, unit, commas(n.unsigned_abs()))
}

fn elapsed_ms(run: &RunRecord) -> u64 {
    let at = |s: &str| DateTime::parse_from_rfc3339(s).ok();
    match (at(&run.started_at), at(&run.finished_at)) {
        (Some(a), Some(b)) => (b - a).num_milliseconds().max(0) as u64,
        _ => 0,
    }
}
