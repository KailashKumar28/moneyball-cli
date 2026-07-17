//! Chat-style message log for the moneyball TUI.
//!
//! Architecture inspired by Codex's `HistoryCell` design (openai/codex):
//!
//! - Each piece of transcript content is a `ChatCell` (trait object).
//!   A cell exposes `display_lines(width)` so it renders wrapped lines
//!   that fit the terminal width.
//! - The "active" cell (the streaming assistant message) is mutated in
//!   place as model deltas arrive - we don't append a new cell per delta.
//! - Concrete cell types implement the trait and carry their own data;
//!   a `Box<dyn ChatCell>` in the log lets a heterogeneous scrollback render
//!   uniformly via virtual dispatch.
//! - Timestamp/status badges live inside each cell, so the renderer
//!   stays a simple loop.
//!
//! What we deliberately do NOT do (yet) vs codex:
//! - markdown rendering (use plain text)
//! - hyperlinks / OSC8
//! - animation ticks (we redraw on every input event instead)
//! - transcript overlay (Ctrl+T)

use chrono::Local;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Trait every cell in the chat log implements.
///
/// Width-aware: cells take the available column count in and return
/// wrapped lines. The renderer never has to know about a cell's width
/// beyond passing it through.
pub trait ChatCell: std::fmt::Debug + Send + Sync {
    /// Render the cell's content as lines for the given viewport width.
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    /// Number of viewport rows the cell will occupy at the given width.
    /// Default implementation counts the lines that `display_lines`
    /// returns. Cells are expected to wrap their own content via
    /// their `display_lines` so this matches what the renderer will draw.
    fn desired_height(&self, width: u16) -> u16 {
        self.display_lines(width).len() as u16
    }
}

/// Cell types. Each is a separate struct so they can have different data
/// and behavior. The log stores `Box<dyn ChatCell>`.
pub mod cells {
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

    /// Reserved for a future width-aware brief cell. Currently unused -
    /// the /brief path pushes a pre-formatted ToolResult. Kept as a struct
    /// + impl so the cell enum is stable for the future swap.
    #[derive(Debug, Clone)]
    pub struct BriefView;

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
                ToolStatus::Pending => "\u{25CB}", // circle
                ToolStatus::Running => "\u{25D0}", // half-filled
                ToolStatus::Done => "\u{25CF}",    // filled
                ToolStatus::Failed => "\u{2716}",  // x mark
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
            for ln in &self.output {
                // Tool output is pre-formatted (aligned tables); never word-wrap
                // it - word-wrapping collapses runs of spaces and destroys column
                // alignment. Clip to the panel width instead, keeping whitespace.
                out.push(Line::from(Span::styled(
                    format!("{}{}", indent, clip(ln, max_w)),
                    Style::default().fg(if self.success {
                        Color::Gray
                    } else {
                        Color::Red
                    }),
                )));
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

    /// BriefView cell - renders a brief as chat lines adapted to the
    /// current column width. At wide widths, one product per line (table).
    /// At narrow widths, three lines per product (name, metrics, KPIs).
    impl ChatCell for BriefView {
        fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
            // Stub - the live /brief path uses ToolResult with the
            // multi-line format from format_brief_as_lines. The Brief cell
            // will swap in once we wire a width-aware brief view.
            vec![]
        }
    }
}

/// Cell enum used by the log. We store as `Box<dyn ChatCell>` so the
/// renderer can iterate uniformly; we also keep an enum here for
/// convenience when we need to mutate the streaming assistant cell.
#[derive(Debug, Clone)]
pub enum Cell {
    UserPrompt(cells::UserPrompt),
    AssistantText(cells::AssistantText),
    ToolCall(cells::ToolCall),
    ToolResult(cells::ToolResult),
    System(cells::System),
    /// Reserved for a future width-aware brief view cell. Currently
    /// unused in the production path - the /brief handler pushes a
    /// pre-formatted ToolResult. Kept as a variant so the cell enum is
    /// stable and we can swap in a real BriefView implementation later.
    BriefPlaceholder,
}

impl Cell {
    /// Wrap into a boxed trait object for the log. Cheap clone via Arc-like
    /// internal; we keep it simple with `Box` for v1.
    pub fn into_boxed(self) -> Box<dyn ChatCell> {
        match self {
            Cell::UserPrompt(c) => Box::new(c),
            Cell::AssistantText(c) => Box::new(c),
            Cell::ToolCall(c) => Box::new(c),
            Cell::ToolResult(c) => Box::new(c),
            Cell::System(c) => Box::new(c),
            Cell::BriefPlaceholder => Box::new(cells::System(String::new())),
        }
    }
}

impl ChatCell for Cell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        match self {
            Cell::UserPrompt(c) => c.display_lines(width),
            Cell::AssistantText(c) => c.display_lines(width),
            Cell::ToolCall(c) => c.display_lines(width),
            Cell::ToolResult(c) => c.display_lines(width),
            Cell::System(c) => c.display_lines(width),
            Cell::BriefPlaceholder => vec![],
        }
    }
    fn desired_height(&self, width: u16) -> u16 {
        match self {
            Cell::UserPrompt(c) => c.desired_height(width),
            Cell::AssistantText(c) => c.desired_height(width),
            Cell::ToolCall(c) => c.desired_height(width),
            Cell::ToolResult(c) => c.desired_height(width),
            Cell::System(c) => c.desired_height(width),
            Cell::BriefPlaceholder => 0,
        }
    }
}

/// The chat scrollback. `scroll = 0` means pinned to bottom (the most
/// recent cell is the last line visible). Positive `scroll` means the
/// user has scrolled up by `scroll` lines.
#[derive(Debug, Default)]
pub struct ChatLog {
    pub cells: Vec<Cell>,
    pub scroll: u16,
    /// Highest useful scroll offset (total lines - viewport), computed
    /// by the last render. Mutations clamp against it so Home/held-Up
    /// can never park the offset thousands of lines past the top
    /// (where Down appears dead). Interior-mutable: render takes &self.
    max_scroll: std::cell::Cell<u16>,
}

impl ChatLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn push(&mut self, c: Cell) {
        self.cells.push(c);
        self.scroll = 0; // auto-pin to bottom on new content
    }

    /// Append a streaming delta to the most recent assistant cell, or
    /// create a new one if there's no streaming cell active.
    pub fn append_assistant(&mut self, chunk: &str) {
        match self.cells.last_mut() {
            Some(Cell::AssistantText(c)) if c.streaming => c.text.push_str(chunk),
            _ => self.cells.push(Cell::AssistantText(cells::AssistantText {
                text: chunk.to_string(),
                streaming: true,
            })),
        }
    }

    /// Mark the most recent streaming assistant cell as complete.
    pub fn finish_streaming(&mut self) {
        if let Some(Cell::AssistantText(c)) = self.cells.last_mut() {
            c.streaming = false;
        }
    }

    /// Push a tool call + result pair (call first, result immediately after).
    pub fn push_tool(
        &mut self,
        name: &str,
        args: &str,
        output: Vec<String>,
        success: bool,
        duration_ms: u64,
    ) {
        self.cells.push(Cell::ToolCall(cells::ToolCall {
            name: name.into(),
            args: args.into(),
            status: if success {
                cells::ToolStatus::Done
            } else {
                cells::ToolStatus::Failed
            },
        }));
        self.cells.push(Cell::ToolResult(cells::ToolResult {
            name: name.into(),
            output,
            success,
            duration_ms,
        }));
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self
            .scroll
            .saturating_add(n)
            .min(self.max_scroll.get());
    }
    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }
    pub fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
    }
    pub fn scroll_to_top(&mut self) {
        self.scroll = self.max_scroll.get();
    }

    /// Flip the most recent Running tool cell to Done/Failed - the
    /// ToolEnd half of the ARCHITECTURE 6b cell contract.
    pub fn finalize_tool(&mut self, ok: bool) {
        for cell in self.cells.iter_mut().rev() {
            if let Cell::ToolCall(c) = cell {
                if matches!(c.status, cells::ToolStatus::Running) {
                    c.status = if ok {
                        cells::ToolStatus::Done
                    } else {
                        cells::ToolStatus::Failed
                    };
                }
                return;
            }
        }
    }

    /// Render the full log within `height` rows for the given viewport
    /// width. Returns the lines (top-to-bottom).
    ///
    /// `scroll` behavior: if 0, we align the BOTTOM of the rendered region
    /// to the bottom of the log. If > 0, we offset upward by `scroll`
    /// logical lines.
    pub fn render(&self, width: u16, height: u16) -> Vec<Line<'static>> {
        if height == 0 {
            return vec![];
        }
        // Compute every cell's height and flatten to logical lines so
        // scroll can be expressed in line units (cheap).
        let mut flat: Vec<Line<'static>> = Vec::new();
        let mut breaks: Vec<usize> = vec![0]; // index into flat where each cell starts
        for cell in &self.cells {
            let lines = cell.display_lines(width);
            // Add a blank separator line between cells (but not before the first).
            if !flat.is_empty() {
                flat.push(Line::from(""));
            }
            for ln in lines {
                flat.push(ln);
            }
            breaks.push(flat.len());
        }
        // Pick the visible window. scroll=0 -> show last `height` lines.
        let total = flat.len();
        self.max_scroll.set(
            total
                .saturating_sub(height as usize)
                .min(u16::MAX as usize) as u16,
        );
        let end = total.saturating_sub(self.scroll as usize);
        let start = end.saturating_sub(height as usize);
        flat.into_iter().skip(start).take(height as usize).collect()
    }
}

// ---------- helpers ----------

/// Clip a pre-formatted line to `width` columns, preserving internal whitespace
/// (word-wrapping would collapse space runs). Adds an ellipsis when cut.
fn clip(s: &str, width: usize) -> String {
    if width == 0 || s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn fmt_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
