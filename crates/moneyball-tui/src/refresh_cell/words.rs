//! A finished run in words: the header's few ("finished without CRM")
//! and the verdict sentence under the steps.

use ratatui::style::Color;

use moneyball_core::refresh::CANCELLED;
use moneyball_core::schema::{Outcome, RunRecord, RunStep, StepStatus};

use super::INDENT;
use crate::widgets::{fg, gray, Para, FAIL};

pub(super) fn cancelled(s: &RunStep) -> bool {
    s.error_kind.as_deref() == Some(CANCELLED)
}

pub(super) fn message(s: &RunStep) -> String {
    s.message.clone().unwrap_or_default()
}

fn step<'a>(run: &'a RunRecord, name: &str) -> Option<&'a RunStep> {
    run.steps.iter().find(|s| s.name == name)
}

/// The header's few words for a degraded run.
pub(super) fn gaps(run: &RunRecord) -> String {
    if run.steps.iter().any(cancelled) {
        return "cancelled".into();
    }
    let no_crm = step(run, "report").is_some_and(|s| s.error_kind.as_deref() == Some("no_crm"));
    if no_crm || step(run, "crm").is_some_and(|s| s.status == StepStatus::Failed) {
        return "finished without CRM".into();
    }
    match step(run, "meta").and_then(|s| s.counts.get("products_failed")) {
        Some(1) => "one product missing".into(),
        Some(n) => format!("{} products missing", n),
        None => "finished with gaps".into(),
    }
}

/// The first sentence after a run that did not go cleanly: what happened,
/// then what it means for the numbers.
pub(super) fn verdict(run: &RunRecord, last_good: Option<&str>, day: &str) -> Option<Para> {
    let (head, rest, color) = match run.outcome {
        Outcome::Ok | Outcome::Running => return None,
        Outcome::Failed => (
            format!("{} Nothing was written today.", FAIL),
            last_good
                .map(|d| format!(" Your last good data ({}) is untouched.", d))
                .unwrap_or_default(),
            Color::Red,
        ),
        Outcome::Degraded => {
            let failed =
                |name: &str| step(run, name).is_some_and(|s| s.status == StepStatus::Failed);
            let (head, rest) = if let Some(at) = run.steps.iter().position(cancelled) {
                (
                    format!("! Cancelled after step {}.", at),
                    " What finished is kept; /refresh runs the rest.".to_string(),
                )
            } else if step(run, "report").is_some_and(|s| s.error_kind.as_deref() == Some("no_crm"))
            {
                (
                    "! The report went out without CRM.".to_string(),
                    // "Complete" only when it is: a 4-of-5 pull sits above.
                    if step(run, "meta").is_some_and(|s| s.status == StepStatus::Done) {
                        " Meta numbers are complete; verdicts are withheld and its CRM columns \
                         read 0 - unknown, not zero."
                    } else {
                        " Verdicts are withheld and its CRM columns read 0 - unknown, not zero."
                    }
                    .to_string(),
                )
            } else if failed("crm") {
                (
                    "! CRM was not refreshed.".to_string(),
                    " The report uses the CRM data this snapshot already had.".to_string(),
                )
            } else if step(run, "meta").is_some_and(|s| s.status == StepStatus::Partial) {
                (
                    format!("! Some products have no data for {}.", day),
                    " The others are complete.".to_string(),
                )
            } else if failed("report") {
                (
                    "! No report was written.".to_string(),
                    " The data is in; /report builds one from it.".to_string(),
                )
            } else {
                (
                    "! Some creatives, leads or ad sets were not captured.".to_string(),
                    " Spend and Meta leads are complete.".to_string(),
                )
            };
            (head, rest, Color::Yellow)
        }
    };
    Some(
        Para::new(INDENT, INDENT + 2)
            .run(head, fg(color))
            .run(rest, gray()),
    )
}
