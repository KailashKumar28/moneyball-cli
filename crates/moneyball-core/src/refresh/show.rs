//! How a refresh reads on screen, shared by the shell (`moneyball
//! refresh`) and the TUI's refresh cell so the two never disagree about
//! what a step is called, what its status word is, or how its counts read.

use crate::schema::{RunStep, StepStatus};
use crate::text::commas;

/// Under a step carried over from today's earlier pull.
pub const REUSED_NOTE: &str = "from today's earlier pull (--force pulls Meta again)";

/// The day a refresh started now fills, for headers ("Wed 07 Oct 2026"):
/// yesterday, since the snapshot day itself is never in a window.
pub fn data_day_label() -> String {
    (chrono::Local::now().date_naive() - chrono::Duration::days(1))
        .format("%a %d %b %Y")
        .to_string()
}

/// The steps every refresh reports, in order.
pub const STEPS: [&str; 4] = ["meta", "assets", "crm", "report"];

pub fn step_label(name: &str) -> &str {
    match name {
        "meta" => "Meta insights",
        "assets" => "creatives and leads",
        "crm" => "CRM",
        "report" => "daily report",
        other => other,
    }
}

/// The status column: done / failed / skipped, "4 of 5" for a pull that
/// missed products, "no CRM" for a report written without CRM data.
pub fn status_word(s: &RunStep) -> String {
    match s.status {
        StepStatus::Done => "done".into(),
        StepStatus::Failed => "failed".into(),
        StepStatus::Skipped => "skipped".into(),
        StepStatus::Partial => match (s.name.as_str(), s.error_kind.as_deref()) {
            ("report", Some("no_crm")) => "no CRM".into(),
            ("meta", _) => match (s.counts.get("products"), s.counts.get("products_failed")) {
                (Some(ok), Some(bad)) => format!("{} of {}", ok, ok + bad),
                _ => "partial".into(),
            },
            _ => "partial".into(),
        },
    }
}

/// A step's counts in reading order: "5 products, 997 rows".
pub fn counts_detail(s: &RunStep) -> String {
    let order: &[&str] = match s.name.as_str() {
        "meta" => &["products", "rows"],
        "assets" => &["creatives", "leads", "adsets"],
        "crm" => &["tickets", "pages"],
        "report" => &["creatives"],
        _ => &[],
    };
    order
        .iter()
        .filter_map(|k| s.counts.get(*k).map(|v| format!("{} {}", commas(*v), k)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// "18.2s", "1m 21s"; empty when the step took no measured time.
pub fn duration(ms: u64) -> String {
    match ms {
        0 => String::new(),
        ms if ms < 60_000 => format!("{:.1}s", ms as f64 / 1000.0),
        ms => format!("{}m {:02}s", ms / 60_000, ms % 60_000 / 1000),
    }
}

/// One plain line per step for the shell: label, status, counts, time;
/// the step's message (a failure's whole reason, a skip's why) goes on
/// the next line, never cut short, and a carried-over step says so.
pub fn step_line(n: usize, s: &RunStep) -> String {
    let line = format!(
        "  {} {:<20} {:<7} {:<40} {:>6}",
        n,
        step_label(&s.name),
        status_word(s),
        counts_detail(s),
        duration(s.ms)
    );
    let mut out = line.trim_end().to_string();
    if let Some(m) = &s.message {
        out.push_str(&format!("\n      {}", m));
    }
    if s.reused && s.status != StepStatus::Skipped {
        out.push_str(&format!("\n      {}", REUSED_NOTE));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(name: &str, status: StepStatus, ms: u64, counts: &[(&str, u64)]) -> RunStep {
        RunStep {
            name: name.into(),
            status,
            ms,
            counts: counts.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            error_kind: None,
            message: None,
            fix: None,
            reused: false,
        }
    }

    #[test]
    fn step_line_keeps_columns_and_puts_the_reason_below() {
        let meta = step(
            "meta",
            StepStatus::Done,
            64_300,
            &[("rows", 1104), ("products", 5)],
        );
        assert_eq!(
            step_line(1, &meta),
            "  1 Meta insights        done    5 products, 1,104 rows                   1m 04s"
        );
        let mut crm = step("crm", StepStatus::Failed, 900, &[]);
        crm.message = Some("CRM rejected the credentials (HTTP 401)".into());
        assert_eq!(
            step_line(3, &crm),
            "  3 CRM                  failed                                             0.9s\n      CRM rejected the credentials (HTTP 401)"
        );
    }

    #[test]
    fn partial_steps_say_what_is_missing() {
        let meta = step(
            "meta",
            StepStatus::Partial,
            1,
            &[("products", 4), ("products_failed", 1)],
        );
        assert_eq!(status_word(&meta), "4 of 5");
        let mut report = step("report", StepStatus::Partial, 1, &[]);
        report.error_kind = Some("no_crm".into());
        assert_eq!(status_word(&report), "no CRM");
        assert_eq!(duration(81_000), "1m 21s");
    }
}
