//! Shared visual elements: the logo, the context bar, and `Para`, the
//! word-wrapping styled paragraph the landing and the refresh cell share.
//!
//! All views (setup wizard, chat) render the same logo + context bar at the
//! top so the user always sees where they are and how fresh the data is.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use moneyball_core::health::{Health, State, STALE_DAYS};

pub const BRAND: &str = "\u{25C6}"; // black diamond
pub const OK: &str = "\u{25CF}"; // black circle
pub const LOAD: &str = "\u{25D0}"; // circle with left half black
pub const IDLE: &str = "\u{25CB}"; // white circle
pub const DOT: &str = "  \u{00B7}  ";

pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}
pub fn gray() -> Style {
    Style::default().fg(Color::Gray)
}
pub fn fg(c: Color) -> Style {
    Style::default().fg(c)
}
pub fn bold(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

/// Top-of-screen logo. Two visual bands: brand mark + tagline.
pub fn logo() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(BRAND, bold(Color::Cyan)),
            Span::raw(" "),
            Span::styled("MONEYBALL", bold(Color::Cyan)),
            Span::styled(DOT, dim()),
            Span::styled("read-only Meta-ads advisor", gray()),
        ]),
        Line::from(""), // spacer line so the logo doesn't crowd the context bar
    ]
}

/// The context bar: the workspace, then how fresh its data is - the data
/// day, the CRM day and one coloured state - in place of a bare "ready".
/// `health` is None before a workspace exists (the setup wizard).
pub fn context_line(
    workspace: &str,
    health: Option<&Health>,
    refreshing: bool,
    next: Option<String>,
    width: u16,
) -> Line<'static> {
    let mut tail: Vec<Span<'static>> = Vec::new();
    match health {
        None => tail.push(Span::styled(
            format!("{}{} first-run setup", DOT, IDLE),
            dim(),
        )),
        Some(h) if h.state == State::NoData && !refreshing => {
            tail.push(Span::styled(format!("{}{} no data", DOT, IDLE), dim()))
        }
        Some(h) => {
            if let (Some(day), Some(age)) = (h.data_day(), h.age_days()) {
                let (value, style) = if age >= STALE_DAYS {
                    (
                        format!("{}, pulled {} days ago", day.format("%d %b"), age),
                        fg(Color::Yellow),
                    )
                } else {
                    (day.format("%a %d %b").to_string(), gray())
                };
                tail.push(Span::styled(format!("{}data ", DOT), dim()));
                tail.push(Span::styled(value, style));
                let crm = match (h.crm_connected, h.crm_day()) {
                    (false, _) => ("not connected".to_string(), fg(Color::Yellow)),
                    (true, None) => ("none".to_string(), fg(Color::Yellow)),
                    (true, Some(d)) if h.crm_snapshot == h.snapshot => {
                        (d.format("%d %b").to_string(), gray())
                    }
                    (true, Some(d)) => (d.format("%d %b").to_string(), fg(Color::Yellow)),
                };
                tail.push(Span::styled(format!("{}CRM ", DOT), dim()));
                tail.push(Span::styled(crm.0, crm.1));
            }
            tail.push(Span::styled(DOT, dim()));
            let (glyph, label, color) = if refreshing {
                (LOAD, "refreshing".to_string(), Color::Cyan)
            } else {
                let color = match h.state {
                    State::Fresh => Color::Green,
                    State::Failed => Color::Red,
                    State::NoData => Color::DarkGray,
                    _ => Color::Yellow,
                };
                (OK, h.label(), color)
            };
            tail.push(Span::styled(format!("{} {}", glyph, label), fg(color)));
            if let Some(next) = next {
                tail.push(Span::styled(format!("{}{}", DOT, next), dim()));
            }
        }
    }
    // The health is the point of the bar: when it would not fit, the
    // workspace path shortens to its last folder, then leaves the bar.
    let tail_w: usize = tail.iter().map(|s| s.content.chars().count()).sum();
    let lead = "  \u{2514}\u{2500} workspace: ";
    let room = (width as usize).saturating_sub(lead.chars().count() + tail_w);
    let home = moneyball_core::config::home_dir();
    let home = home.as_deref().and_then(|h| h.to_str()).unwrap_or_default();
    let mut path = match workspace.strip_prefix(home) {
        Some(rest) if !home.is_empty() => format!("~{}", rest),
        _ => workspace.to_string(),
    };
    if path.chars().count() > room {
        path = std::path::Path::new(workspace)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(path);
    }
    if path.chars().count() > room {
        if let Some(first) = tail.first_mut() {
            first.content = first
                .content
                .trim_start_matches([' ', '\u{00B7}'])
                .to_string()
                .into();
        }
        let mut spans = vec![Span::styled("  \u{2514}\u{2500} ", dim())];
        spans.extend(tail);
        return Line::from(spans);
    }
    let mut spans = vec![Span::styled(lead, dim()), Span::styled(path, gray())];
    spans.extend(tail);
    Line::from(spans)
}

/// The day's numbers in one line, as the report has them: spend, Meta
/// leads, then CRM leads, qualified and cost per qualified when it has CRM.
pub fn day_numbers(report: &moneyball_core::schema::CreativeReport) -> String {
    let k = &report.portfolio;
    let mut parts = vec![
        format!(
            "spend Rs.{}",
            moneyball_core::refresh::commas(k.spend.round() as u64)
        ),
        format!("Meta leads {}", k.funnel.m_leads),
    ];
    if report.source.crm_present {
        parts.push(format!("CRM leads {}", k.funnel.l_leads));
        parts.push(format!("qualified {}", k.funnel.qualified));
        if let Some(c) = k.cost_per_qualified {
            parts.push(format!(
                "Rs.{} per qualified",
                moneyball_core::refresh::commas(c.round() as u64)
            ));
        }
    } else {
        parts.push("no CRM in this report".into());
    }
    parts.join(DOT)
}

/// What a fix command does, said once next to it.
pub fn fix_does(fix: &str) -> &'static str {
    if fix == "/setup" {
        "paste a fresh Meta token; products and settings stay"
    } else if fix.contains("crm secret") {
        "in a shell; it asks for the new token"
    } else if fix.contains("crm connect") {
        "in a shell; it walks you through connecting your CRM"
    } else {
        "in a shell"
    }
}

/// A paragraph of styled runs that word-wraps to the width it is drawn
/// at: `indent` columns lead the first line, `hang` the rest. Spaces
/// inside a line are kept (the "  .  " separators), a word wider than the
/// line (a path) is split, and an empty paragraph is a blank line.
#[derive(Debug, Clone, Default)]
pub struct Para {
    pub runs: Vec<(String, Style)>,
    pub indent: usize,
    pub hang: usize,
}

impl Para {
    pub fn new(indent: usize, hang: usize) -> Self {
        Para {
            runs: Vec::new(),
            indent,
            hang,
        }
    }

    pub fn run(mut self, text: impl Into<String>, style: Style) -> Self {
        self.runs.push((text.into(), style));
        self
    }

    pub fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let width = (width as usize).max(self.indent.max(self.hang) + 10);
        let mut out = Vec::new();
        let mut cur = vec![Span::raw(" ".repeat(self.indent))];
        let mut used = self.indent;
        let mut fresh = true;
        for (text, style) in &self.runs {
            for tok in tokens(text) {
                if !fresh && used + tok.trim_end().chars().count() > width {
                    out.push(end_line(&mut cur));
                    cur.push(Span::raw(" ".repeat(self.hang)));
                    used = self.hang;
                    fresh = true;
                }
                let mut rest: Vec<char> = if fresh {
                    tok.trim_start().chars().collect()
                } else {
                    tok.chars().collect()
                };
                // Only a word wider than a whole line gets here unfitted
                // (its trailing spaces may overhang; a break drops them).
                while used + rest.iter().rev().skip_while(|c| **c == ' ').count() > width {
                    let head: String = rest.drain(..width - used).collect();
                    cur.push(Span::styled(head, *style));
                    out.push(end_line(&mut cur));
                    cur.push(Span::raw(" ".repeat(self.hang)));
                    used = self.hang;
                }
                if !rest.is_empty() {
                    used += rest.len();
                    fresh = false;
                    cur.push(Span::styled(rest.into_iter().collect::<String>(), *style));
                }
            }
        }
        out.push(end_line(&mut cur));
        out
    }
}

/// Close a wrapped line, dropping the spaces a break leaves at its end.
fn end_line(cur: &mut Vec<Span<'static>>) -> Line<'static> {
    let n = cur.len();
    if let Some(last) = cur.last_mut() {
        let kept = last.content.trim_end().to_string();
        last.content = kept.into();
        if last.content.is_empty() && n > 1 {
            cur.pop();
        }
    }
    Line::from(std::mem::take(cur))
}

/// "  .  CRM leads 42" -> ["  .  ", "CRM ", "leads ", "42"]: each word
/// keeps the spaces after it; leading spaces stay with the first word.
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c != ' ' && cur.ends_with(' ') && !cur.trim().is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn para_wraps_words_keeps_separators_and_splits_paths() {
        let p = Para::new(2, 4)
            .run("! Late.", bold(Color::Yellow))
            .run(" spend Rs.9  \u{00B7}  qualified 7", gray());
        assert_eq!(
            text(&p.lines(24)),
            vec!["  ! Late. spend Rs.9  \u{00B7}", "    qualified 7"]
        );
        let path = Para::new(0, 2).run("report /a/b/c/d/e/f/g/h/i/j/k", gray());
        assert_eq!(
            text(&path.lines(12)),
            vec!["report", "  /a/b/c/d/e", "  /f/g/h/i/j", "  /k"]
        );
        assert_eq!(text(&Para::new(6, 6).lines(40)), vec![""]);
    }
}
