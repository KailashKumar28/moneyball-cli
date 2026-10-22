//! Chat-style message log for the moneyball TUI.
//!
//! Architecture after Codex's `HistoryCell` (openai/codex): each piece of
//! the transcript is a cell (the `Cell` enum, its types in cells.rs) that
//! renders itself as wrapped lines for a width, so the renderer stays a
//! simple loop. The streaming assistant message is mutated in place as
//! deltas arrive, never a new cell per delta. Assistant text renders as
//! markdown (markdown.rs); the screen redraws every 100ms tick.

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

pub mod cells;

/// One transcript entry. An enum, not a trait object, so the streaming
/// assistant cell and a running tool's cell can be found and changed.
#[derive(Debug, Clone)]
pub enum Cell {
    UserPrompt(cells::UserPrompt),
    AssistantText(cells::AssistantText),
    ToolCall(cells::ToolCall),
    ToolResult(cells::ToolResult),
    System(cells::System),
    Rich(cells::Rich),
    /// A /refresh run, updated in place as its steps land.
    Refresh(Box<crate::refresh_cell::RefreshCell>),
}

impl ChatCell for Cell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        match self {
            Cell::UserPrompt(c) => c.display_lines(width),
            Cell::AssistantText(c) => c.display_lines(width),
            Cell::ToolCall(c) => c.display_lines(width),
            Cell::ToolResult(c) => c.display_lines(width),
            Cell::System(c) => c.display_lines(width),
            Cell::Rich(c) => c.display_lines(width),
            Cell::Refresh(c) => c.display_lines(width),
        }
    }
    fn desired_height(&self, width: u16) -> u16 {
        match self {
            Cell::UserPrompt(c) => c.desired_height(width),
            Cell::AssistantText(c) => c.desired_height(width),
            Cell::ToolCall(c) => c.desired_height(width),
            Cell::ToolResult(c) => c.desired_height(width),
            Cell::System(c) => c.desired_height(width),
            Cell::Rich(c) => c.desired_height(width),
            Cell::Refresh(c) => c.desired_height(width),
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

    /// A dim one-line answer under a command that could not run now, so
    /// its prompt never sits in the transcript unanswered.
    pub fn note(&mut self, text: impl Into<String>) {
        self.push(Cell::System(cells::System(text.into())));
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
        self.scroll = self.scroll.saturating_add(n).min(self.max_scroll.get());
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
        self.max_scroll
            .set(total.saturating_sub(height as usize).min(u16::MAX as usize) as u16);
        let end = total.saturating_sub(self.scroll as usize);
        let start = end.saturating_sub(height as usize);
        flat.into_iter().skip(start).take(height as usize).collect()
    }
}

// ---------- helpers ----------

fn fmt_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
