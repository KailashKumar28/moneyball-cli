//! The transcript's cell types - what the user typed, the advisor's
//! text, tool calls and results, system notes, rich paragraphs - each
//! rendering itself for a width. Split from chat.rs (size cap).

use super::*;

/// User prompt (what they typed at the input bar).
#[derive(Debug, Clone)]
pub struct UserPrompt {
    pub text: String,
    pub at: chrono::DateTime<Local>,
}

/// Assistant text. While `streaming = true` the renderer tacks a
/// blinking caret onto the last line.
#[derive(Debug, Clone)]
pub struct AssistantText {
    pub text: String,
    pub streaming: bool,
}

/// Tool invocation: `moneyball brief`, `moneyball funnel Namma Mane`, etc.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: String,
    pub status: ToolStatus,
}

/// Output of a tool call. Rendered immediately under the matching
/// `ToolCall` cell by the renderer (paired by index).
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub name: String,
    pub output: Vec<String>,
    pub success: bool,
    pub duration_ms: u64,
}

/// System message: welcome, info banners, errors.
#[derive(Debug, Clone)]
pub struct System(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl ToolStatus {
    pub fn icon(self) -> &'static str {
        match self {
            ToolStatus::Pending => crate::widgets::IDLE,
            ToolStatus::Running => crate::widgets::LOAD,
            ToolStatus::Done => crate::widgets::OK,
            ToolStatus::Failed => crate::widgets::FAIL,
        }
    }
    pub fn color(self) -> Color {
        match self {
            ToolStatus::Pending => Color::DarkGray,
            ToolStatus::Running => Color::Yellow,
            ToolStatus::Done => Color::Green,
            ToolStatus::Failed => Color::Red,
        }
    }
}

impl ChatCell for UserPrompt {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        // `>` prompt glyph + user text, then a timestamp on the next line.
        vec![
            Line::from(vec![
                Span::styled(
                    "\u{276F} ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(self.text.clone(), Style::default().fg(Color::White)),
            ]),
            Line::from(Span::styled(
                format!("    {}", self.at.format("%H:%M:%S")),
                Style::default().fg(Color::DarkGray),
            )),
        ]
    }
}

impl ChatCell for AssistantText {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        // Markdown-aware render (codex/Claude Code pattern): markers
        // become styling, never raw ** or backticks on screen.
        let mut out = crate::markdown::render(&self.text, width, "  ");
        if self.streaming {
            let caret = Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            );
            match out.last_mut() {
                Some(last) => last.spans.push(caret),
                None => out.push(Line::from(vec![Span::raw("  "), caret])),
            }
        }
        if out.is_empty() {
            out.push(Line::from(Span::raw("  ")));
        }
        out
    }
}

impl ChatCell for ToolCall {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let arg = if self.args.is_empty() {
            String::new()
        } else {
            format!(" {}", self.args)
        };
        vec![Line::from(vec![
            Span::styled("  \u{23BF} ", Style::default().fg(Color::DarkGray)),
            Span::styled(self.status.icon(), Style::default().fg(self.status.color())),
            Span::styled(" moneyball ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                self.name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(arg, Style::default().fg(Color::Gray)),
        ])]
    }
}

impl ChatCell for ToolResult {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let indent = "      ";
        let max_w = width.saturating_sub(indent.len() as u16).max(20) as usize;
        let style = Style::default().fg(if self.success {
            Color::Gray
        } else {
            Color::Red
        });
        for ln in &self.output {
            // Tool output is pre-formatted (aligned tables): wrap it by
            // columns, keeping its spaces, and indent the overflow.
            let pieces = moneyball_core::text::wrap_columns(ln, max_w, max_w - 2);
            for (i, piece) in pieces.into_iter().enumerate() {
                let lead = if i == 0 { indent } else { "        " };
                out.push(Line::from(Span::styled(
                    format!("{}{}", lead, piece),
                    style,
                )));
            }
        }
        // 0 means "not measured" (replayed sessions) - print no
        // duration rather than a fabricated "(0ms)".
        if self.duration_ms > 0 {
            out.push(Line::from(Span::styled(
                format!("      ({})", fmt_duration(self.duration_ms)),
                Style::default().fg(if self.success {
                    Color::Green
                } else {
                    Color::Red
                }),
            )));
        }
        out
    }
}

impl ChatCell for System {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        // Multi-line system text (e.g. the ASCII logo) - split on '\n'
        // so each logical row is its own rendered line. Otherwise the
        // whole string collapses to one wrapped visual line.
        let mut out = Vec::new();
        for ln in self.0.split('\n') {
            out.push(Line::from(Span::styled(
                if ln.is_empty() {
                    "  ".to_string()
                } else {
                    format!("  {}", ln)
                },
                Style::default().fg(Color::DarkGray),
            )));
        }
        out
    }
}

/// Styled paragraphs that wrap to the width (the landing).
#[derive(Debug, Clone)]
pub struct Rich(pub Vec<crate::widgets::Para>);

impl ChatCell for Rich {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.iter().flat_map(|p| p.lines(width)).collect()
    }
}
