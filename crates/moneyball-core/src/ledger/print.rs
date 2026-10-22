//! The ledger as text, for the shell and the TUI cell: open recs with
//! their checks, the settled ones, and how often each lever was right.

use chrono::Local;

use super::{name, Entry, LEVERS};
use crate::config::AppConfig;
use crate::error::Result;
use crate::schema::RecState;
use crate::text::commas;

/// `moneyball ledger`: the ledger caught up with the newest snapshot
/// (MB_AGENT=1: the entries as JSON).
pub fn run(cfg: &AppConfig) -> Result<()> {
    let today = Local::now().date_naive();
    super::catch_up(cfg, today)?;
    let entries = super::load(cfg);
    match cfg.agent {
        true => println!("{}", serde_json::to_string(&entries)?),
        false => print!("{}", text(&entries)),
    }
    Ok(())
}

/// Closed recs listed before the rest are only counted.
const CLOSED_SHOWN: usize = 10;
const DOT: &str = "  \u{00B7}  ";

pub fn text(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "LEDGER - no recommendations yet. Ask the advisor what to change: \
                it records each one here with the number it predicts.\n"
            .into();
    }
    let (open, closed): (Vec<&Entry>, Vec<&Entry>) = entries.iter().partition(|e| e.open());
    let mut out = format!("LEDGER - {} open, {} closed\n", open.len(), closed.len());
    for e in &open {
        out.push_str(&block(e));
    }
    if !closed.is_empty() {
        out.push_str("\nclosed, newest first\n");
        for e in closed.iter().rev().take(CLOSED_SHOWN) {
            out.push_str(&block(e));
        }
        if closed.len() > CLOSED_SHOWN {
            out.push_str(&format!("\nand {} older\n", closed.len() - CLOSED_SHOWN));
        }
    }
    out.push_str(&hit_rate(entries));
    out
}

/// One rec: what and where, the change itself, then where it stands.
fn block(e: &Entry) -> String {
    let r = &e.rec;
    let p = &r.prediction;
    let mut head = format!(
        "{}  {}{}{}{}{}",
        e.id,
        name(e.state),
        DOT,
        r.product,
        DOT,
        r.lever
    );
    if r.level != "product" {
        head.push_str(&format!("{}{}", DOT, r.entity_name));
    }
    let predicts = format!("{} {} to {}", p.metric, p.direction, number(p.target));
    let check = e
        .verify_by()
        .map_or(p.verify_by.clone(), |d| d.format("%d %b").to_string());
    let note = e.note.as_deref().unwrap_or("");
    let stands = match e.state {
        RecState::Proposed => format!("predicts {}, checked after {}", predicts, check),
        RecState::Applied => format!(
            "applied {} ({}); predicts {}, checked after {}",
            e.applied
                .map_or("-".into(), |d| d.format("%d %b").to_string()),
            note,
            predicts,
            check
        ),
        RecState::Ignored => format!("ignored; it predicted {}", predicts),
        RecState::Expired => format!("never applied by {}; it predicted {}", check, predicts),
        _ => format!("predicted {}; measured {}", predicts, note),
    };
    format!("\n{}\n  {}\n  {}\n", head, r.action, stands)
}

/// Rupees and counts in hundreds and up with commas, small ratios with
/// a decimal.
pub(super) fn number(v: f64) -> String {
    match v.abs() >= 100.0 {
        true => commas(v.round().max(0.0) as u64),
        false => format!("{:.1}", v),
    }
}

/// "budget 3 of 4 right" per lever with a verdict; inconclusive ones say
/// nothing about the advisor, so they are not counted.
fn hit_rate(entries: &[Entry]) -> String {
    let parts: Vec<String> = LEVERS
        .iter()
        .filter_map(|(lever, _)| {
            let of = |s: RecState| {
                entries
                    .iter()
                    .filter(|e| e.rec.lever == *lever && e.state == s)
                    .count()
            };
            let (right, wrong) = (of(RecState::Confirmed), of(RecState::Refuted));
            (right + wrong > 0).then(|| format!("{} {} of {} right", lever, right, right + wrong))
        })
        .collect();
    match parts.is_empty() {
        true => "\nno prediction has been checked yet.\n".into(),
        false => format!("\nhit rate: {}\n", parts.join(", ")),
    }
}
