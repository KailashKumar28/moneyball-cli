//! Scoreboard history (history/scoreboard.csv) + per-product trend
//! strings. Split from the compute module (size cap).

use std::collections::HashMap;
use std::path::Path;

use super::{HistoryRow, ProductRow};

pub fn load_history(path: &Path) -> Vec<HistoryRow> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(raw.as_bytes());
    let mut out = Vec::new();
    for row in rdr.records().flatten() {
        let product = row.get(0).unwrap_or("").to_string();
        let qualified: f64 = row.get(2).unwrap_or("0").parse().unwrap_or(0.0);
        if !product.is_empty() {
            out.push(HistoryRow { product, qualified });
        }
    }
    out
}

pub(super) fn trend_rows_for(
    history: &[HistoryRow],
    rows: &[ProductRow],
) -> HashMap<String, Vec<f64>> {
    let mut out: HashMap<String, Vec<f64>> = HashMap::new();
    for r in rows {
        let h: Vec<f64> = history
            .iter()
            .rev()
            .filter(|h| h.product == r.product)
            .take(7)
            .map(|h| h.qualified)
            .collect();
        // history is most-recent-first; we want oldest->newest like mb.py
        let mut v = h;
        v.reverse();
        out.insert(r.product.clone(), v);
    }
    out
}

pub(super) fn apply_trends(rows: &mut [ProductRow], trends: HashMap<String, Vec<f64>>) {
    for r in rows.iter_mut() {
        if let Some(v) = trends.get(&r.product) {
            r.trend = if v.is_empty() {
                "-".into()
            } else {
                v.iter()
                    .map(|q| format!("{:.0}", q))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
        }
    }
}
