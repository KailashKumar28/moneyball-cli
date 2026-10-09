//! The landing (frames R1, R5): what the first screen says about the data
//! before any number does - stale, a failed morning, missing CRM, missing
//! products - and, when all is well, yesterday in one line and its report.

use std::path::PathBuf;

use chrono::{DateTime, Duration, NaiveDate};
use ratatui::style::{Color, Modifier, Style};

use moneyball_core::health::{Health, State};
use moneyball_core::refresh::{self as r};
use moneyball_core::report::daily;
use moneyball_core::schema::{CreativeReport, RunRecord, StepStatus, Trigger};
use moneyball_core::text::commas;
use moneyball_core::AppConfig;

use crate::widgets::{bold, day_numbers, dim, fg, fix_does, gray, Para, DOT, FAIL};

const DAY: &str = "%a %d %b";
const NOT_CONNECTED: &str =
    "No CRM is connected, so CRM leads, qualified and every verdict are off.";

/// The landing's paragraphs, and the report ctrl+o opens (when there is
/// one). `scheduled`: a morning refresh is installed.
pub(crate) fn landing(
    cfg: &AppConfig,
    h: &Health,
    scheduled: bool,
) -> (Vec<Para>, Option<PathBuf>) {
    let day = h
        .data_day()
        .map(|d| d.format(DAY).to_string())
        .unwrap_or_default();
    let (mut out, fixes) = story(h, &day, scheduled);
    // States whose day still has numbers worth showing under the story.
    let whole_day = matches!(
        h.state,
        State::Fresh | State::Partial { .. } | State::CrmMissing | State::Pinned
    );
    // Without numbers below, the fix and the next command stand apart.
    if !whole_day {
        out.push(Para::default());
    }
    for fix in &fixes {
        out.push(
            Para::new(4, 10)
                .run("fix:  ", gray())
                .run(fix.clone(), bold(Color::Cyan))
                .run(format!("   {}", fix_does(fix)), gray()),
        );
    }
    out.extend(next_command(h, !fixes.is_empty(), !scheduled && !whole_day));
    if !whole_day {
        return (out, None);
    }
    if !out.is_empty() {
        out.push(Para::default());
    }
    let report = h
        .snap_date
        .and_then(|s| daily(cfg, &s.format("%Y-%m-%d").to_string()));
    out.extend(day_block(cfg, h, &day, report.as_ref()));
    (out, report.map(|(_, html)| html))
}

/// What is wrong, since when, and the fixes the run records name.
fn story(h: &Health, day: &str, scheduled: bool) -> (Vec<Para>, Vec<String>) {
    let mut out = Vec::new();
    let mut fixes: Vec<String> = Vec::new();
    let run = h.run.as_ref();
    let fixes_of =
        |r: &RunRecord| -> Vec<String> { r.steps.iter().filter_map(|s| s.fix.clone()).collect() };
    match h.state {
        State::NoData => out.push(headline(
            "! No data yet.",
            " Nothing has been pulled for this workspace.",
            Color::Yellow,
        )),
        State::Stale => {
            let mut rest = match h.refreshed_at {
                Some(t) => format!(
                    " Last refresh: {}{}.",
                    t.format("%a %d %b, %-I:%M %P"),
                    run.map_or(String::new(), |r| format!(" ({})", how(r)))
                ),
                None => String::new(),
            };
            if let Some(snap) = h.snap_date {
                rest.push_str(&missed(snap, h.today));
            }
            let age = h.age_days().unwrap_or(0);
            out.push(headline(
                &format!("! Your data is {} days old.", age),
                &rest,
                Color::Yellow,
            ));
            if let Some(why) = crm_behind(h) {
                out.push(Para::new(4, 4).run(why, gray()));
            }
            if !scheduled {
                out.push(Para::new(4, 4).run("Nothing refreshes on its own yet.", gray()));
            }
        }
        State::Failed => {
            let failed = h.failed_today.as_ref();
            let step = failed.and_then(|f| f.steps.iter().find(|s| s.status == StepStatus::Failed));
            let rest = step.map_or(String::new(), |s| {
                format!(
                    " {}: {}",
                    r::step_label(&s.name),
                    s.message.clone().unwrap_or_default()
                )
            });
            out.push(headline(
                &format!("{} Today's refresh failed.", FAIL),
                &rest,
                Color::Red,
            ));
            // A first refresh that failed has no earlier data to speak of.
            if h.snap_date.is_some() {
                out.push(Para::new(4, 4).run(
                    format!("Your last good data ({}) is still here.", day),
                    gray(),
                ));
            }
            fixes.extend(failed.map(fixes_of).unwrap_or_default());
        }
        State::CrmMissing => {
            let crm = run.and_then(|r| r.steps.iter().find(|s| s.name == "crm"));
            let rest = match crm {
                _ if !h.crm_connected => format!(" {}", NOT_CONNECTED),
                Some(s) if s.status == StepStatus::Failed => format!(
                    " {}.",
                    s.message
                        .as_deref()
                        .unwrap_or_default()
                        .trim_end_matches('.')
                ),
                _ => " CRM was not pulled into this snapshot, so every verdict is withheld."
                    .to_string(),
            };
            out.push(headline(
                &format!("! {} has no CRM data.", day),
                &rest,
                Color::Yellow,
            ));
            fixes.extend(run.map(fixes_of).unwrap_or_default());
            if !h.crm_connected && fixes.is_empty() {
                fixes.push("moneyball crm connect".into());
            }
        }
        State::Partial { have, of } => {
            let meta = run.and_then(|r| r.steps.iter().find(|s| s.name == "meta"));
            let rest = meta
                .and_then(|s| s.message.clone())
                .map(|m| format!(" {}", m))
                .unwrap_or_default();
            out.push(headline(
                &format!("! {} of {} products have data for {}.", have, of, day),
                &rest,
                Color::Yellow,
            ));
        }
        State::Pinned => {
            let newest = h
                .newest_snap_date
                .filter(|n| Some(*n) != h.snap_date)
                .and_then(|n| n.pred_opt())
                .map(|n| format!(" The newest data is {}.", n.format(DAY)))
                .unwrap_or_default();
            out.push(
                Para::new(2, 4)
                    .run(
                        format!("Viewing {} (--date).", day),
                        Style::default().add_modifier(Modifier::BOLD),
                    )
                    .run(newest, gray()),
            );
        }
        State::Fresh => {}
    }
    (out, fixes)
}

/// The one command that brings the data in, under any fix ("then"), and
/// /schedule beneath it when nothing refreshes on its own.
fn next_command(h: &Health, after_fix: bool, offer_schedule: bool) -> Vec<Para> {
    let (cmd, does) = match h.state {
        State::Fresh | State::Pinned => return Vec::new(),
        State::Partial { .. } => ("/refresh --force", "pulls Meta again for every product"),
        State::CrmMissing if h.age_days() == Some(0) => (
            "/refresh",
            "reuses today's Meta pull: only CRM and the report run again",
        ),
        _ => (
            "/refresh",
            "pulls Meta, then CRM, then writes yesterday's report (about 2 minutes)",
        ),
    };
    // "then" sits under the fix's command, as in the refresh cell; both
    // commands pad to one width so their sentences line up.
    let (at, word) = if after_fix {
        (10, "then ")
    } else {
        (4, "next:  ")
    };
    let w = if offer_schedule {
        cmd.len().max(9)
    } else {
        cmd.len()
    };
    let mut out = vec![Para::new(at, at)
        .run(word, gray())
        .run(format!("{:<w$}", cmd), bold(Color::Cyan))
        .run(format!("   {}", does), gray())];
    if offer_schedule {
        let under = at + word.len();
        out.push(
            Para::new(under, under)
                .run(format!("{:<w$}", "/schedule"), bold(Color::Cyan))
                .run(
                    "   so it happens every morning before you open this",
                    gray(),
                ),
        );
    }
    out
}

/// R5: the day, when and how it was refreshed, its numbers, the change
/// against the last comparable day, and the report.
fn day_block(
    cfg: &AppConfig,
    h: &Health,
    day: &str,
    report: Option<&(CreativeReport, PathBuf)>,
) -> Vec<Para> {
    let mut when = h
        .refreshed_at
        .map(|t| moneyball_core::status::when(t, h.today))
        .unwrap_or_default();
    if let Some(run) = &h.run {
        let ms = elapsed_ms(run);
        let took = if ms > 0 {
            format!(", {}", r::duration(ms))
        } else {
            String::new()
        };
        when.push_str(&format!(" ({}{})", how(run), took));
    }
    let mut out = vec![Para::new(2, 4)
        .run(
            day.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )
        .run(
            if when.is_empty() {
                String::new()
            } else {
                format!("{}refreshed {}", DOT, when)
            },
            dim(),
        )];
    let Some((rep, html)) = report else {
        out.push(Para::new(2, 4).run(
            "no daily report for this day yet - /refresh writes one",
            gray(),
        ));
        return out;
    };
    out.push(Para::new(2, 4).run(day_numbers(rep), gray()));
    // Only against a day within the last week: an older one says little.
    let day_of = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    let recent = |d: &&str| match (day_of(d), day_of(&rep.report_date)) {
        (Some(a), Some(b)) => (b - a).num_days() <= 7,
        _ => false,
    };
    let prior = rep
        .prior_date
        .as_deref()
        .filter(recent)
        .and_then(|d| daily(cfg, d));
    if let Some((before, _)) = prior {
        out.push(versus(rep, &before));
    }
    out.push(
        Para::new(2, 10)
            .run("report  ", gray())
            .run(html.display().to_string(), fg(Color::Cyan)),
    );
    out.push(Para::new(10, 10).run(
        format!("ctrl+o open it{}/status for the last 14 mornings", DOT),
        dim(),
    ));
    out
}

/// "vs Wed 07 Oct:  qualified +2   cost per qualified -Rs.3,532", or
/// spend and Meta leads when either day has no CRM.
fn versus(now: &CreativeReport, before: &CreativeReport) -> Para {
    let day = NaiveDate::parse_from_str(&before.window.until, "%Y-%m-%d")
        .map(|d| d.format(DAY).to_string())
        .unwrap_or_else(|_| before.window.until.clone());
    let (a, b) = (&now.portfolio, &before.portfolio);
    let good = |better: bool, d: i64| match d {
        0 => gray(),
        _ if better => fg(Color::Green),
        _ => fg(Color::Red),
    };
    let mut p = Para::new(2, 4).run(format!("vs {}:  ", day), gray());
    if now.source.crm_present && before.source.crm_present {
        let dq = a.funnel.qualified as i64 - b.funnel.qualified as i64;
        p = p
            .run("qualified ", gray())
            .run(signed(dq, ""), good(dq > 0, dq));
        if let (Some(x), Some(y)) = (a.cost_per_qualified, b.cost_per_qualified) {
            let dc = x.round() as i64 - y.round() as i64;
            p = p
                .run("   cost per qualified ", gray())
                .run(signed(dc, "Rs."), good(dc < 0, dc));
        }
    } else {
        let ds = a.spend.round() as i64 - b.spend.round() as i64;
        let dl = a.funnel.m_leads as i64 - b.funnel.m_leads as i64;
        p = p
            .run("spend ", gray())
            .run(signed(ds, "Rs."), gray())
            .run("   Meta leads ", gray())
            .run(signed(dl, ""), good(dl > 0, dl));
    }
    p
}

fn signed(n: i64, unit: &str) -> String {
    let sign = if n < 0 { "-" } else { "+" };
    format!("{}{}{}", sign, unit, commas(n.unsigned_abs()))
}

fn headline(head: &str, rest: &str, color: Color) -> Para {
    Para::new(2, 4)
        .run(head.to_string(), bold(color))
        .run(rest.to_string(), gray())
}

fn how(run: &RunRecord) -> &'static str {
    match run.trigger {
        Trigger::Schedule => "scheduled",
        Trigger::Manual => "manual",
    }
}

/// " 9 mornings missed, 29 Sep to 07 Oct." - the days between the
/// snapshot and today that never got a refresh.
fn missed(snap: NaiveDate, today: NaiveDate) -> String {
    let (from, to) = (snap + Duration::days(1), today - Duration::days(1));
    match (to - from).num_days() + 1 {
        n if n < 1 => String::new(),
        1 => format!(" 1 morning missed, {}.", from.format("%d %b")),
        n => format!(
            " {} mornings missed, {} to {}.",
            n,
            from.format("%d %b"),
            to.format("%d %b")
        ),
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

fn elapsed_ms(run: &RunRecord) -> u64 {
    let at = |s: &str| DateTime::parse_from_rfc3339(s).ok();
    match (at(&run.started_at), at(&run.finished_at)) {
        (Some(a), Some(b)) => (b - a).num_milliseconds().max(0) as u64,
        _ => 0,
    }
}

#[cfg(test)]
#[path = "landing_tests.rs"]
mod tests;
