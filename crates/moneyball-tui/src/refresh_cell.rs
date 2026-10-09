//! The /refresh cell (frames R2, R3, S1 to S3): a live step table while
//! the run goes, then the day's numbers, the report, and what to do about
//! anything missing. Drawn from structured state, not pre-formatted text,
//! so the columns stay aligned and the detail column wraps at any width.

use std::time::Instant;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use moneyball_core::refresh::{self as r, STEPS};
use moneyball_core::schema::{Outcome, RunRecord, RunStep, StepStatus};

use crate::chat::ChatCell;
use crate::widgets::{bold, dim, fg, fix_does, gray, Para, DOT, LOAD, OK};

const INDENT: usize = 6;
const LABEL_W: usize = 23;
const STATUS_W: usize = 9;
const DETAIL_W: usize = 53;
/// Where the detail column starts: the indent, "1  ", label, status.
const DETAIL_AT: usize = INDENT + 3 + LABEL_W + STATUS_W;
const FAIL: &str = "\u{2716}";

#[derive(Debug, Clone)]
pub enum Row {
    Pending,
    Running,
    Done(RunStep),
}

#[derive(Debug, Clone)]
pub enum End {
    Finished {
        run: Box<RunRecord>,
        ms: u64,
        /// The day's numbers, from the report this run wrote.
        numbers: Option<String>,
        report: Option<String>,
        /// The data day on screen before the run, kept when it fails.
        last_good: Option<String>,
    },
    CouldNotStart(String),
}

#[derive(Debug, Clone)]
pub struct RefreshCell {
    /// The data day this run fills ("Thu 08 Oct").
    pub day: String,
    pub started: Instant,
    /// One row per refresh::STEPS, in order.
    pub rows: Vec<Row>,
    pub end: Option<End>,
}

impl RefreshCell {
    pub fn new(day: String) -> Self {
        RefreshCell {
            day,
            started: Instant::now(),
            rows: vec![Row::Pending; STEPS.len()],
            end: None,
        }
    }

    pub fn set(&mut self, name: &str, row: Row) {
        if let Some(i) = STEPS.iter().position(|s| *s == name) {
            self.rows[i] = row;
        }
    }

    fn header(&self) -> Line<'static> {
        let (glyph, color, suffix) = match &self.end {
            None => {
                let s = self.started.elapsed().as_secs();
                (
                    LOAD,
                    Color::Yellow,
                    format!("{}{:02}:{:02} so far", DOT, s / 60, s % 60),
                )
            }
            Some(End::CouldNotStart(_)) => (FAIL, Color::Red, format!("{}could not start", DOT)),
            Some(End::Finished { run, .. }) => match run.outcome {
                Outcome::Ok | Outcome::Running => (OK, Color::Green, String::new()),
                Outcome::Failed => {
                    let at = run
                        .steps
                        .iter()
                        .position(|s| s.status == StepStatus::Failed)
                        .map_or(1, |i| i + 1);
                    (FAIL, Color::Red, format!("{}stopped at step {}", DOT, at))
                }
                Outcome::Degraded => (OK, Color::Yellow, format!("{}{}", DOT, gaps(run))),
            },
        };
        Line::from(vec![
            Span::styled("  \u{23BF} ", dim()),
            Span::styled(glyph, fg(color)),
            Span::styled(" moneyball ", dim()),
            Span::styled("refresh", bold(Color::Cyan)),
            Span::styled(format!("   for {}{}", self.day, suffix), dim()),
        ])
    }

    fn row_lines(&self, i: usize, row: &Row, width: u16) -> Vec<Line<'static>> {
        let lead = format!(
            "{}{}  {:<LABEL_W$}",
            " ".repeat(INDENT),
            i + 1,
            r::step_label(STEPS[i])
        );
        let s = match row {
            Row::Pending => {
                let waits = if STEPS[i] == "report" { "CRM" } else { "Meta" };
                return vec![Line::from(Span::styled(
                    format!("{}waits for {}", lead, waits),
                    dim(),
                ))];
            }
            Row::Running => {
                return vec![Line::from(vec![
                    Span::styled(lead, gray()),
                    Span::styled("running", fg(Color::Yellow)),
                ])]
            }
            Row::Done(s) => s,
        };
        // A report without CRM says so in its status word ("no CRM") and
        // the verdict below: its row keeps its counts, like a done step.
        let no_crm = s.error_kind.as_deref() == Some("no_crm");
        let (word_style, detail, timed) = match s.status {
            StepStatus::Done => (fg(Color::Green), r::counts_detail(s), true),
            StepStatus::Partial if no_crm => (fg(Color::Yellow), r::counts_detail(s), true),
            StepStatus::Partial => (fg(Color::Yellow), message(s), false),
            StepStatus::Failed => (fg(Color::Red), message(s), false),
            StepStatus::Skipped => (dim(), message(s), false),
        };
        let skipped = s.status == StepStatus::Skipped;
        let text_style = if skipped { dim() } else { gray() };
        let time = r::duration(s.ms);
        let timed = timed && !time.is_empty();
        let room = (width as usize).saturating_sub(DETAIL_AT).max(16);
        let detail_w = if timed {
            DETAIL_W.min(room.saturating_sub(9)).max(16)
        } else {
            room
        };
        let pieces = moneyball_core::text::wrap_columns(&detail, detail_w, detail_w);
        let mut first = vec![
            Span::styled(lead, text_style),
            Span::styled(format!("{:<STATUS_W$}", r::status_word(s)), word_style),
        ];
        if timed {
            first.push(Span::styled(
                format!("{:<detail_w$}", pieces[0]),
                text_style,
            ));
            first.push(Span::styled(format!("({})", time), fg(Color::Green)));
        } else {
            first.push(Span::styled(pieces[0].clone(), text_style));
        }
        let mut out = vec![Line::from(first)];
        let under = |text: String, style: Style| {
            Line::from(Span::styled(
                format!("{}{}", " ".repeat(DETAIL_AT), text),
                style,
            ))
        };
        out.extend(pieces[1..].iter().map(|p| under(p.clone(), text_style)));
        // A finished step's note (a reused pull) sits dim under its counts.
        if s.status == StepStatus::Done {
            if let Some(m) = &s.message {
                let note = moneyball_core::text::wrap_columns(m, room, room);
                out.extend(note.into_iter().map(|p| under(p, dim())));
            }
        }
        out
    }

    fn footer(&self, end: &End) -> Vec<Para> {
        let mut paras = vec![Para::default()];
        let (run, ms, numbers, report, last_good) = match end {
            End::CouldNotStart(err) => {
                paras.push(
                    Para::new(INDENT, INDENT + 2)
                        .run(
                            format!("{} The refresh could not start:", FAIL),
                            fg(Color::Red),
                        )
                        .run(format!(" {}", err), gray()),
                );
                return paras;
            }
            End::Finished {
                run,
                ms,
                numbers,
                report,
                last_good,
            } => (run, ms, numbers, report, last_good),
        };
        if let Some(p) = verdict(run, last_good.as_deref(), &self.day) {
            paras.push(p);
        }
        let fixes: Vec<&String> = run.steps.iter().filter_map(|s| s.fix.as_ref()).collect();
        for fix in &fixes {
            paras.push(
                Para::new(INDENT, INDENT + 6)
                    .run("fix:  ", gray())
                    .run(fix.to_string(), bold(Color::Cyan))
                    .run(format!("   {}", fix_does(fix)), gray()),
            );
        }
        if !fixes.is_empty() {
            let meta_in = run
                .steps
                .iter()
                .any(|s| s.name == "meta" && s.status != StepStatus::Failed);
            paras.push(
                Para::new(INDENT + 6, INDENT + 6)
                    .run("then ", gray())
                    .run("/refresh", bold(Color::Cyan))
                    .run(
                        if meta_in {
                            "   reuses this morning's Meta pull: only CRM and the report run again"
                        } else {
                            ""
                        },
                        gray(),
                    ),
            );
        }
        if paras.len() > 1 && (numbers.is_some() || report.is_some()) {
            paras.push(Para::default());
        }
        if let Some(n) = numbers {
            paras.push(
                Para::new(INDENT, INDENT + 2)
                    .run(
                        format!("{}:", self.day),
                        Style::default().add_modifier(Modifier::BOLD),
                    )
                    .run(format!("  {}", n), gray()),
            );
        }
        if let Some(path) = report {
            paras.push(
                Para::new(INDENT, INDENT + 8)
                    .run("report  ", gray())
                    .run(path.clone(), fg(Color::Cyan)),
            );
            paras.push(Para::new(INDENT + 8, INDENT + 8).run(
                format!("ctrl+o open it{}/brief or /funnel <product> to dig in", DOT),
                dim(),
            ));
        }
        paras.push(
            Para::new(INDENT, INDENT).run(format!("({})", r::duration(*ms)), fg(Color::Green)),
        );
        paras
    }
}

impl ChatCell for RefreshCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut out = vec![self.header(), Line::from("")];
        for (i, row) in self.rows.iter().enumerate() {
            out.extend(self.row_lines(i, row, width));
        }
        if let Some(end) = &self.end {
            for p in self.footer(end) {
                out.extend(p.lines(width));
            }
        }
        out
    }
}

fn message(s: &RunStep) -> String {
    s.message.clone().unwrap_or_default()
}

fn step<'a>(run: &'a RunRecord, name: &str) -> Option<&'a RunStep> {
    run.steps.iter().find(|s| s.name == name)
}

/// The header's few words for a degraded run.
fn gaps(run: &RunRecord) -> String {
    if run
        .steps
        .iter()
        .any(|s| s.message.as_deref() == Some("cancelled"))
    {
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
fn verdict(run: &RunRecord, last_good: Option<&str>, day: &str) -> Option<Para> {
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
            let (head, rest) = if let Some(at) = run
                .steps
                .iter()
                .position(|s| s.message.as_deref() == Some("cancelled"))
            {
                (
                    format!("! Cancelled after step {}.", at),
                    " What finished is kept; /refresh runs the rest.".to_string(),
                )
            } else if step(run, "report").is_some_and(|s| s.error_kind.as_deref() == Some("no_crm"))
            {
                (
                    "! The report went out without CRM.".to_string(),
                    " Meta numbers are complete; verdicts are withheld and its CRM columns \
                     read 0 - unknown, not zero."
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
