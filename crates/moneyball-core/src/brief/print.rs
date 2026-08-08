//! Brief presentation - the fixed-width CLI table + feasibility print.
//! Split from the compute module (size cap): compute returns typed
//! rows; this file owns how they look on stdout.

use super::{staleness_warning, ProductRow, ProductRowsAndFeasibility};

pub fn print_brief(r: &ProductRowsAndFeasibility) {
    println!(
        "BRIEF  snapshot {}  (7d window; config.json goals)",
        r.snapshot_date
    );
    if let Some(warn) = staleness_warning(&r.snapshot_date) {
        println!("{}", warn);
    }
    println!("{}", format_brief_table(&r.rows));
    let f = &r.feasibility;
    println!();
    println!(
        "FEASIBILITY  portfolio {:.1} q/day at Rs.{}/day = Rs.{}/qualified - goal {:.0}/day",
        f.tot_q_per_day,
        comma(f.tot_spend_per_day as i64),
        comma(f.cur_rpq as i64),
        f.tot_goal_per_day
    );
    if let Some(req) = f.required_at_cur {
        println!(
            "  required spend at CURRENT efficiency : Rs.{}/day ({:.1}x today)",
            comma(req as i64),
            req as f64 / f.tot_spend_per_day.max(1) as f64
        );
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        println!(
            "  required spend at BEST-OBSERVED Rs.{}/q : Rs.{}/day ({:.1}x today)",
            comma(b as i64),
            comma(req as i64),
            req as f64 / f.tot_spend_per_day.max(1) as f64
        );
    }
    let n = f.open_debt.len();
    let suffix = if f.open_debt.is_empty() {
        String::new()
    } else {
        format!(" ({})", f.open_debt.join(", "))
    };
    println!("  open setup debt: {}{}", n, suffix);
}

fn comma(n: i64) -> String {
    let s = n.abs().to_string();
    let negative = n < 0;
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 3);
    for (i, &b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(b',');
        }
        out.push(b);
    }
    out.reverse();
    let mut s = String::from_utf8(out).unwrap();
    if negative {
        s.insert(0, '-');
    }
    s
}

/// Cell-renderer closure type - used by format_brief_table to keep the
/// column-spec vec a readable single-typed alias instead of inline.
type BriefCell = Box<dyn Fn(&ProductRow) -> String>;
type BriefSpec<'a> = (&'a str, BriefCell);

pub fn format_brief_table(rows: &[ProductRow]) -> String {
    let cols: Vec<BriefSpec<'_>> = vec![
        ("product", Box::new(|r| r.product.clone())),
        ("spend_per_day", Box::new(|r| r.spend_per_day.to_string())),
        ("m7d", Box::new(|r| r.m7d.to_string())),
        ("l7d", Box::new(|r| r.l7d.to_string())),
        ("q7d", Box::new(|r| r.q7d.to_string())),
        ("q_per_day", Box::new(|r| fmt_f64(r.q_per_day, 2))),
        (
            "rs_per_q",
            Box::new(|r| {
                r.rs_per_q
                    .map(|x| x.to_string())
                    .unwrap_or_else(|| "-".into())
            }),
        ),
        (
            "l_to_q",
            Box::new(|r| {
                r.l_to_q
                    .map(|x| format!("{:.1}", x))
                    .unwrap_or_else(|| "-".into())
            }),
        ),
        ("goal", Box::new(|r| fmt_f64(r.goal, 0))),
        ("gap", Box::new(|r| fmt_f64(r.gap, 1))),
        ("q_last7_by_day", Box::new(|r| r.trend.clone())),
    ];
    let headers: Vec<&str> = cols.iter().map(|(h, _)| *h).collect();
    let _width = |s: &str| s.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    let mut rendered: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for r in rows {
        let row: Vec<String> = cols.iter().map(|(_, f)| f(r)).collect();
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
        rendered.push(row);
    }
    let mut lines = Vec::new();
    lines.push(
        headers
            .iter()
            .enumerate()
            .map(|(i, h)| h.pad_to_width(widths[i]))
            .collect::<Vec<_>>()
            .join("  "),
    );
    lines.push(lines[0].chars().map(|_| '-').collect());
    for row in rendered {
        lines.push(
            row.iter()
                .enumerate()
                .map(|(i, c)| c.pad_to_width(widths[i]))
                .collect::<Vec<_>>()
                .join("  "),
        );
    }
    lines.join("\n")
}

fn fmt_f64(x: f64, decimals: usize) -> String {
    let scaled = (x * 10f64.powi(decimals as i32)).round() as i64;
    let sign = if scaled < 0 { "-" } else { "" };
    let abs = scaled.unsigned_abs() as f64 / 10f64.powi(decimals as i32);
    format!("{}{}", sign, abs)
}

trait PadToWidth {
    fn pad_to_width(&self, w: usize) -> String;
}
impl PadToWidth for str {
    fn pad_to_width(&self, w: usize) -> String {
        if self.len() >= w {
            self.to_string()
        } else {
            format!("{}{}", self, " ".repeat(w - self.len()))
        }
    }
}
