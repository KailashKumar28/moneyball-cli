//! The /refresh cell (frames R2, R3, S1 to S3): a live step table while
//! the run goes, then the day's numbers, the report, and what to do about
//! anything missing. Drawn from structured state, not pre-formatted text,
//! so the columns stay aligned and the detail column wraps at any width.

use std::time::Instant;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use moneyball_core::refresh::{self as r, REUSED_NOTE, STEPS};
use moneyball_core::schema::{Outcome, RunRecord, RunStep, StepStatus};

use crate::chat::ChatCell;
use crate::widgets::{bold, dim, fg, gray, tint_color, tint_mark, Para, DOT, FAIL, LOAD, OK};

const INDENT: usize = 6;
const LABEL_W: usize = 23;
const STATUS_W: usize = 9;
const DETAIL_W: usize = 53;
/// Where the detail column starts: the indent, "1  ", label, status.
const DETAIL_AT: usize = INDENT + 3 + LABEL_W + STATUS_W;

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
    /// The worker died mid-run (a panic): the rows show how far it got.
    Stopped(String),
}

#[derive(Debug, Clone)]
pub struct RefreshCell {
    /// The data day this run fills ("Thu 08 Oct").
    pub day: String,
    pub started: Instant,
    /// One row per refresh::STEPS, in order.
    pub rows: Vec<Row>,
    pub end: Option<End>,
    /// No morning refresh installed: a good run ends by offering one.
    pub offer_schedule: bool,
}

impl RefreshCell {
    pub fn new(day: String, offer_schedule: bool) -> Self {
        RefreshCell {
            day,
            started: Instant::now(),
            rows: vec![Row::Pending; STEPS.len()],
            end: None,
            offer_schedule,
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
            Some(End::Stopped(_)) => (FAIL, Color::Red, format!("{}stopped unexpectedly", DOT)),
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
                Outcome::Degraded => (OK, Color::Yellow, format!("{}{}", DOT, r::gaps(run))),
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
                let waits = match STEPS[i] {
                    "meta" => "waiting",
                    "report" => "waits for CRM",
                    _ => "waits for Meta",
                };
                return vec![Line::from(Span::styled(
                    format!("{}{}", lead, waits),
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
        // Notes sit dim under the detail: a finished step's message, and
        // where a carried-over step came from.
        let mut notes: Vec<&str> = Vec::new();
        if s.status == StepStatus::Done {
            notes.extend(s.message.as_deref());
        }
        if s.reused && !skipped {
            notes.push(REUSED_NOTE);
        }
        for n in notes {
            let lines = moneyball_core::text::wrap_columns(n, room, room);
            out.extend(lines.into_iter().map(|p| under(p, dim())));
        }
        out
    }

    fn footer(&self, end: &End) -> Vec<Para> {
        let mut paras = vec![Para::default()];
        let (run, ms, numbers, report, last_good) = match end {
            End::CouldNotStart(err) | End::Stopped(err) => {
                let head = if matches!(end, End::Stopped(_)) {
                    "The refresh stopped unexpectedly:"
                } else {
                    "The refresh could not start:"
                };
                paras.push(
                    Para::new(INDENT, INDENT + 2)
                        .run(format!("{} {}", FAIL, head), fg(Color::Red))
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
        paras.extend(r::verdict(run, last_good.as_deref(), &self.day).map(verdict));
        let fixes = r::fixes(run);
        for fix in &fixes {
            paras.push(
                Para::new(INDENT, INDENT + 6)
                    .run("fix:  ", gray())
                    .run(fix.command.clone(), bold(Color::Cyan))
                    .run(format!("   {}", fix.does), gray()),
            );
        }
        if !fixes.is_empty() {
            let then = r::rerun(run);
            let does = if then.does.is_empty() {
                String::new()
            } else {
                format!("   {}", then.does)
            };
            paras.push(
                Para::new(INDENT + 6, INDENT + 6)
                    .run("then ", gray())
                    .run(then.command, bold(Color::Cyan))
                    .run(does, gray()),
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
            paras.push(Para::new(INDENT, INDENT + 8).run("report  ", gray()).run(
                crate::widgets::tilde(std::path::Path::new(path)),
                fg(Color::Cyan),
            ));
            paras.push(Para::new(INDENT + 8, INDENT + 8).run(
                format!("ctrl+o open it{}/brief or /funnel <product> to dig in", DOT),
                dim(),
            ));
        }
        paras.push(
            Para::new(INDENT, INDENT).run(format!("({})", r::duration(*ms)), fg(Color::Green)),
        );
        if self.offer_schedule && run.outcome != Outcome::Failed {
            paras.push(Para::default());
            paras.push(
                Para::new(2, 2)
                    .run("Want this every morning?  ", gray())
                    .run("/schedule", bold(Color::Cyan)),
            );
        }
        paras
    }
}

fn message(s: &RunStep) -> String {
    s.message.clone().unwrap_or_default()
}

/// The verdict as one paragraph: its command reads on as the sentence's
/// second half ("What finished is kept; /refresh runs the rest.").
fn verdict(v: r::RunVerdict) -> Para {
    let rest = match &v.next {
        Some(c) => format!(
            " {}; {} {}.",
            v.rest.trim_end_matches('.'),
            c.command,
            c.does
        ),
        None if v.rest.is_empty() => String::new(),
        None => format!(" {}", v.rest),
    };
    Para::new(INDENT, INDENT + 2)
        .run(
            format!("{}{}", tint_mark(v.tint), v.headline),
            fg(tint_color(v.tint)),
        )
        .run(rest, gray())
}

impl ChatCell for RefreshCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut out = vec![self.header(), Line::from("")];
        // A run that never started has no steps to show.
        if !matches!(self.end, Some(End::CouldNotStart(_))) {
            for (i, row) in self.rows.iter().enumerate() {
                out.extend(self.row_lines(i, row, width));
            }
        }
        if let Some(end) = &self.end {
            for p in self.footer(end) {
                out.extend(p.lines(width));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
