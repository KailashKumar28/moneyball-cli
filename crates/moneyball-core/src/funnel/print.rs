//! Funnel presentation. Split from the compute module (size cap).
//!
//! Two tables from one grid: `view` is what a person reads (frame S4 -
//! who, what it cost, and the call, in about 90 columns), printed by the
//! shell and the TUI cell; `table` is the full analytical one (ids, kill
//! multiple, sufficiency, maturity, learning) the LLM reasons from.

use super::FunnelRow;

const COLS: &[&str] = &[
    "id",
    "name",
    "spend",
    "m",
    "cpl",
    "l",
    "q",
    "v",
    "rs_per_q",
    "l_to_q",
    "kill_mult",
    "kill",
    "sufficient",
    "immature",
    "learning",
];

const VIEW: &[&str] = &[
    "name", "spend", "meta", "cpl", "crm", "qual", "rs_per_q", "verdict",
];

fn opt(v: Option<u64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "-".into())
}

fn cell(r: &FunnelRow, col: &str) -> String {
    match col {
        "id" => r.id.clone(),
        "name" => r.name.chars().take(32).collect(),
        "spend" => r.spend.to_string(),
        "m" => r.m.to_string(),
        "cpl" => opt(r.cpl),
        "l" => r.l.to_string(),
        "q" => r.q.to_string(),
        "v" => r.v.to_string(),
        "rs_per_q" => opt(r.rs_per_q),
        "l_to_q" => r
            .l_to_q
            .map(|x| x.to_string())
            .unwrap_or_else(|| "-".into()),
        "kill_mult" => r.kill_mult.to_string(),
        "kill" => r
            .kill
            .map(|k| k.to_string())
            .unwrap_or_else(|| "no CRM".into()),
        "sufficient" => r.sufficient.to_string(),
        "immature" => r.immature.to_string(),
        "learning" => r.learning.clone(),
        _ => unreachable!(),
    }
}

/// The one call per row, in the order the advisor weighs them: no CRM,
/// then the reasons to wait, then kill, else keep.
pub fn verdict(r: &FunnelRow) -> &'static str {
    match r.kill {
        None => "no CRM",
        Some(_) if !r.sufficient => "wait: low spend",
        Some(_) if r.immature => "wait: immature",
        Some(_) if r.learning == "LEARNING" => "wait: learning",
        Some(true) => "kill",
        Some(false) => "keep",
    }
}

fn view_cell(r: &FunnelRow, col: &str) -> String {
    // Without CRM the CRM columns are unknown, never 0.
    let crm = |v: u64| r.kill.map_or("-".into(), |_| v.to_string());
    match col {
        "meta" => r.m.to_string(),
        "crm" => crm(r.l),
        "qual" => crm(r.q),
        "verdict" => verdict(r).into(),
        other => cell(r, other),
    }
}

/// The full analytical table, mb.py `_tab` style: header, dashed rule,
/// rows. ASCII-only so the TUI can render it verbatim.
pub fn table(rows: &[FunnelRow]) -> String {
    grid(COLS, rows, cell)
}

/// The table a person reads (frame S4).
pub fn view(rows: &[FunnelRow]) -> String {
    grid(VIEW, rows, view_cell)
}

fn grid(cols: &[&str], rows: &[FunnelRow], cell: fn(&FunnelRow, &str) -> String) -> String {
    if rows.is_empty() {
        return "(no rows)\n".into();
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| cols.iter().map(|c| cell(r, c)).collect())
        .collect();
    // Widths in chars, as `{:<w$}` pads: names carry dashes like U+2013.
    let widths: Vec<usize> = (0..cols.len())
        .map(|i| {
            cells
                .iter()
                .map(|row| row[i].chars().count())
                .chain(std::iter::once(cols[i].len()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |row: &[String]| {
        row.iter()
            .zip(&widths)
            .map(|(c, w)| format!("{:<w$}", c))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let header: Vec<String> = cols.iter().map(|c| c.to_string()).collect();
    let mut out = line(&header);
    let rule = "-".repeat(widths.iter().sum::<usize>() + 2 * (widths.len() - 1));
    out.push('\n');
    out.push_str(&rule);
    out.push('\n');
    for row in &cells {
        out.push_str(&line(row));
        out.push('\n');
    }
    // Every surface prints through here (CLI, TUI cell, the agent's tool),
    // so the reason travels with the withheld verdicts.
    if rows.iter().any(|r| r.kill.is_none()) {
        out.push_str(
            "kill withheld: this snapshot has no CRM data, so qualified counts are unknown, not zero. \
             Run a refresh (moneyball refresh, or /refresh in the app), then this again.\n",
        );
    }
    out
}
