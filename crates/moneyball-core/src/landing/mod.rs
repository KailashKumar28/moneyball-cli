//! The landing in words: what the first screen says about the data before
//! any number does - stale, a failed morning, missing CRM, missing
//! products - and, when all is well, yesterday in one line and its report.
//! Unstyled and serializable: a front-end maps `Tint` to colour, lays the
//! parts out, and presents each `Command` its own way.

mod day;
mod story;

use std::path::PathBuf;

use serde::Serialize;

use crate::config::AppConfig;
use crate::health::{Health, State};
use crate::words::{Command, Tint};

pub use day::{day_numbers, refreshed};

/// How the landing names a day: "Thu 08 Oct".
const DAY: &str = "%a %d %b";

/// Everything the landing says, in reading order.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Landing {
    /// What is wrong and since when; None when the data is fresh.
    pub headline: Option<Headline>,
    /// Quieter sentences under the headline.
    pub details: Vec<String>,
    /// What to do first, as the run records name it.
    pub fixes: Vec<Command>,
    /// The one command that brings the data in (after any fix).
    pub next: Option<Command>,
    /// Offered beneath `next` when nothing refreshes on its own.
    pub schedule: Option<Command>,
    /// The day on screen, when its numbers are worth showing.
    pub day: Option<DayBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Headline {
    pub tint: Tint,
    pub text: String,
    /// The sentences that follow it in a quieter voice; may be empty.
    pub rest: String,
}

/// The day, when and how it was refreshed, and its report.
#[derive(Debug, Clone, Serialize)]
pub struct DayBlock {
    /// "Thu 08 Oct".
    pub day: String,
    /// "07:30 this morning (scheduled, 1m 21s)"; None when unknown.
    pub refreshed: Option<String>,
    pub report: DayReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DayReport {
    /// No daily report for this day yet: what to say, and what writes one.
    Missing { note: String, next: Command },
    Written {
        numbers: DayNumbers,
        /// The change against the last comparable day within a week.
        versus: Option<Versus>,
        /// The HTML report a client opens.
        path: PathBuf,
    },
}

/// The day's numbers as the report has them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayNumbers {
    pub numbers: Vec<Number>,
    /// Said in place of the CRM numbers when the report has none.
    pub note: Option<String>,
}

impl DayNumbers {
    /// Each number as it reads in a sentence, then the note.
    pub fn phrases(&self) -> Vec<&str> {
        let numbers = self.numbers.iter().map(|n| n.phrase.as_str());
        numbers.chain(self.note.as_deref()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Number {
    /// "spend", "cost per qualified".
    pub label: String,
    /// "Rs.1,234".
    pub value: String,
    /// The two as they read in a line: "Rs.1,234 per qualified".
    pub phrase: String,
}

/// The day against an earlier one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Versus {
    /// The earlier day: "Wed 07 Oct".
    pub day: String,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub label: String,
    pub by: i64,
    /// `by` as it reads: "+2", "-Rs.3,532".
    pub text: String,
    /// Good or Bad for the business; Plain when unchanged or neither.
    pub tint: Tint,
}

/// The landing for this health reading. `scheduled`: a morning refresh
/// is installed. File reads only (the day's report and the one before).
pub fn landing(cfg: &AppConfig, h: &Health, scheduled: bool) -> Landing {
    let day = h
        .data_day()
        .map(|d| d.format(DAY).to_string())
        .unwrap_or_default();
    let mut out = story::tell(h, &day, scheduled);
    out.next = story::next_command(h);
    // States whose day still has numbers worth showing under the story.
    let whole_day = matches!(
        h.state,
        State::Fresh | State::Partial { .. } | State::CrmMissing | State::Pinned
    );
    if whole_day {
        out.day = Some(day::day_block(cfg, h, day));
    } else if !scheduled {
        out.schedule = Some(Command::new(
            "/schedule",
            "so it happens every morning before you open this",
        ));
    }
    out
}

#[cfg(test)]
mod tests;
