//! The landing (frames R1, R5) on screen. What it says is core's
//! (`moneyball_core::landing`); this lays it out and colours it.

use std::path::PathBuf;

use ratatui::style::{Color, Modifier, Style};

use moneyball_core::health::Health;
use moneyball_core::landing::{self as words, DayBlock, DayReport, Headline, Versus};
use moneyball_core::words::{Command, Tint};
use moneyball_core::AppConfig;

use crate::widgets::{bold, dim, fg, gray, tint_color, tint_mark, Para, DOT};

/// The landing's paragraphs, and the report ctrl+o opens (when there is
/// one). `scheduled`: a morning refresh is installed.
pub(crate) fn landing(
    cfg: &AppConfig,
    h: &Health,
    scheduled: bool,
) -> (Vec<Para>, Option<PathBuf>) {
    let l = words::landing(cfg, h, scheduled);
    let mut out: Vec<Para> = l.headline.iter().map(headline).collect();
    out.extend(
        l.details
            .iter()
            .map(|d| Para::new(4, 4).run(d.clone(), gray())),
    );
    // Without numbers below, the fix and the next command stand apart.
    if l.day.is_none() {
        out.push(Para::default());
    }
    for fix in &l.fixes {
        out.push(
            Para::new(4, 10)
                .run("fix:  ", gray())
                .run(fix.command.clone(), bold(Color::Cyan))
                .run(format!("   {}", fix.does), gray()),
        );
    }
    if let Some(next) = &l.next {
        out.extend(next_command(next, !l.fixes.is_empty(), l.schedule.as_ref()));
    }
    let Some(day) = &l.day else {
        return (out, None);
    };
    if !out.is_empty() {
        out.push(Para::default());
    }
    out.extend(day_block(day));
    let report = match &day.report {
        DayReport::Written { path, .. } => Some(path.clone()),
        DayReport::Missing { .. } => None,
    };
    (out, report)
}

fn headline(h: &Headline) -> Para {
    let style = match h.tint {
        Tint::Plain => Style::default().add_modifier(Modifier::BOLD),
        tint => bold(tint_color(tint)),
    };
    Para::new(2, 4)
        .run(format!("{}{}", tint_mark(h.tint), h.text), style)
        .run(spaced(&h.rest), gray())
}

/// A sentence that follows another on the same line.
fn spaced(rest: &str) -> String {
    if rest.is_empty() {
        String::new()
    } else {
        format!(" {}", rest)
    }
}

/// The one command that brings the data in, under any fix ("then"), and
/// /schedule beneath it when nothing refreshes on its own.
fn next_command(next: &Command, after_fix: bool, schedule: Option<&Command>) -> Vec<Para> {
    // "then" sits under the fix's command, as in the refresh cell; both
    // commands pad to one width so their sentences line up.
    let (at, word) = if after_fix {
        (10, "then ")
    } else {
        (4, "next:  ")
    };
    let cmd = &next.command;
    let w = schedule.map_or(cmd.len(), |s| cmd.len().max(s.command.len()));
    let mut out = vec![Para::new(at, at)
        .run(word, gray())
        .run(format!("{:<w$}", cmd), bold(Color::Cyan))
        .run(format!("   {}", next.does), gray())];
    if let Some(s) = schedule {
        let under = at + word.len();
        out.push(
            Para::new(under, under)
                .run(format!("{:<w$}", s.command), bold(Color::Cyan))
                .run(format!("   {}", s.does), gray()),
        );
    }
    out
}

/// R5: the day, when and how it was refreshed, its numbers, the change
/// against the last comparable day, and the report.
fn day_block(d: &DayBlock) -> Vec<Para> {
    let refreshed = d
        .refreshed
        .as_ref()
        .map(|when| format!("{}refreshed {}", DOT, when))
        .unwrap_or_default();
    let mut out = vec![Para::new(2, 4)
        .run(d.day.clone(), Style::default().add_modifier(Modifier::BOLD))
        .run(refreshed, dim())];
    let (numbers, versus_day, path) = match &d.report {
        DayReport::Missing { note, next } => {
            out.push(
                Para::new(2, 4).run(format!("{} - {} {}", note, next.command, next.does), gray()),
            );
            return out;
        }
        DayReport::Written {
            numbers,
            versus,
            path,
        } => (numbers, versus, path),
    };
    out.push(Para::new(2, 4).run(numbers.phrases().join(DOT), gray()));
    out.extend(versus_day.as_ref().map(versus));
    out.push(
        Para::new(2, 10)
            .run("report  ", gray())
            .run(crate::widgets::tilde(path), fg(Color::Cyan)),
    );
    out.push(Para::new(10, 10).run(
        format!("ctrl+o open it{}/status for the last 14 mornings", DOT),
        dim(),
    ));
    out
}

/// "vs Wed 07 Oct:  qualified +2   cost per qualified -Rs.3,532", or
/// spend and Meta leads when either day has no CRM.
fn versus(v: &Versus) -> Para {
    let mut p = Para::new(2, 4).run(format!("vs {}:  ", v.day), gray());
    for (i, c) in v.changes.iter().enumerate() {
        let gap = if i == 0 { "" } else { "   " };
        p = p
            .run(format!("{}{} ", gap, c.label), gray())
            .run(c.text.clone(), fg(tint_color(c.tint)));
    }
    p
}

#[cfg(test)]
#[path = "landing_tests.rs"]
mod tests;
