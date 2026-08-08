//! Funnel presentation - the fixed-width table. Split from the
//! compute module (size cap).

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

fn cell(r: &FunnelRow, col: &str) -> String {
    fn opt(v: Option<u64>) -> String {
        v.map(|x| x.to_string()).unwrap_or_else(|| "-".into())
    }
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
        "kill" => r.kill.to_string(),
        "sufficient" => r.sufficient.to_string(),
        "immature" => r.immature.to_string(),
        "learning" => r.learning.clone(),
        _ => unreachable!(),
    }
}

/// Fixed-width table, mb.py `_tab` style: header, dashed rule, rows.
/// ASCII-only so the TUI can render it verbatim.
pub fn table(rows: &[FunnelRow]) -> String {
    if rows.is_empty() {
        return "(no rows)\n".into();
    }
    let widths: Vec<usize> = COLS
        .iter()
        .map(|c| {
            rows.iter()
                .map(|r| cell(r, c).len())
                .chain(std::iter::once(c.len()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let header: Vec<String> = COLS
        .iter()
        .zip(&widths)
        .map(|(c, w)| format!("{:<width$}", c, width = w))
        .collect();
    let mut out = header.join("  ");
    let rule = "-".repeat(out.len());
    out.push('\n');
    out.push_str(&rule);
    out.push('\n');
    for r in rows {
        let line: Vec<String> = COLS
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{:<width$}", cell(r, c), width = w))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}
