//! Frame rendering: shared chrome (logo + context bar) and the
//! chat view (transcript, input, palette, captions).

use crate::*;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

// Two chrono types collide on import name; alias the plain Utc.

pub(crate) fn render(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    // Layout (top to bottom):
    //   logo (2) | context (1) | body (Min) | commands (0 - hidden) | input (3)
    //
    // For the setup wizard, body is the wizard panel (no commands shown).
    // For the chat view (default), body is the chat log + commands + input.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // logo
            Constraint::Length(1), // context bar
            Constraint::Min(6),    // body
        ])
        .split(area);

    // Logo + context bar.
    f.render_widget(Paragraph::new(crate::widgets::logo()), outer[0]);
    let status = if app.brief.is_some() {
        crate::widgets::Status::Ready
    } else if app.cfg.has_workspace() {
        crate::widgets::Status::NoData
    } else {
        crate::widgets::Status::Idle
    };
    let ctx_line = crate::widgets::context_line(
        &app.cfg.data_root.display().to_string(),
        app.snap_date.as_deref(),
        status,
    );
    f.render_widget(Paragraph::new(ctx_line), outer[1]);

    match &app.view {
        View::Setup(s) => setup::render_setup(f, outer[2], s),
        View::Brief => render_chat_view(f, outer[2], app),
    }
}

/// Chat-style body: scrollable log + compact command hint + input bar.
fn render_chat_view(f: &mut ratatui::Frame, area: Rect, app: &App) {
    // Chat-style bottom stack (matches codex / claude code / pi):
    //   1. inline completion ghost (only when input is `/`-prefixed)
    //   2. thin horizontal separator
    //   3. the input line itself (single bar, no boxed title)
    //   4. one-line keybinding caption
    //   5. status hint when something notable happened
    // Command palette (codex / claude-code pattern): a dropdown of
    // matching commands + descriptions BELOW the input line, arrow-
    // navigable, Enter runs the selected entry. Height adapts to the
    // terminal: fixed rows must never squeeze out the body/chrome.
    let status_h: u16 = if app.status.is_some() { 1 } else { 0 };
    let reserved = 3 + status_h; // separator + input + caption + status
                                 // While the palette is open it outranks transcript rows (the user is
                                 // mid-command); keep >=2 body rows and give the palette the rest.
    let palette_room = area.height.saturating_sub(reserved + 2);
    let palette_h: u16 = (app.completions.len() as u16).min(8).min(palette_room);

    // Body height: as many rows as the content needs, up to whatever the
    // terminal allows after reserving the input area (separator + input +
    // palette + caption + status). Short logs stay compact; long logs use
    // the full screen; anything beyond is reachable via scroll keys.
    let max_body = area.height.saturating_sub(reserved + palette_h).max(5);
    let content_lines = count_chat_lines(app, area.width) as u16;
    let body_h = content_lines.clamp(5, max_body);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(body_h),    // chat scrollback (natural height)
            Constraint::Length(1),         // separator
            Constraint::Length(1),         // input line
            Constraint::Length(palette_h), // command palette (below input)
            Constraint::Length(1),         // keybinding caption
            Constraint::Length(status_h),  // status hint
        ])
        .split(area);
    let sep_idx = 1;
    let input_idx = 2;
    let palette_idx = 3;
    let caption_idx = 4;
    let status_idx = 5;

    // 1. Chat log
    let log_lines: Vec<Line<'static>> = {
        let width = chunks[0].width.max(20);
        let height = chunks[0].height.max(5);
        app.chat.render(width, height)
    };
    f.render_widget(Paragraph::new(log_lines), chunks[0]);

    // 2. Thin horizontal separator (above the input line)
    let width = area.width as usize;
    f.render_widget(
        Paragraph::new(Line::from(sep_str(width))).style(Style::default().fg(Color::DarkGray)),
        chunks[sep_idx],
    );

    // 3. Command palette rows (below the input): `▸ /brief   description`,
    // selected row highlighted. Renders only while a `/` prefix matches.
    if palette_h > 0 {
        // Window the list around the selected row so arrow-navigation
        // never moves the highlight off-screen when the list is taller
        // than the palette area.
        let visible = palette_h as usize;
        let sel = app.completion_idx.unwrap_or(0);
        let start = if sel >= visible { sel + 1 - visible } else { 0 };
        let mut rows: Vec<Line<'static>> = Vec::new();
        for (i, c) in app.completions.iter().enumerate().skip(start).take(visible) {
            let desc = commands::COMMANDS
                .iter()
                .find(|(name, _)| name == c)
                .map(|(_, d)| *d)
                .unwrap_or("");
            let selected = Some(i) == app.completion_idx;
            let (marker, cmd_style, desc_style) = if selected {
                (
                    "\u{25B8} ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::Gray),
                )
            } else {
                (
                    "  ",
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                )
            };
            rows.push(Line::from(vec![
                Span::styled(format!("  {}", marker), cmd_style),
                Span::styled(format!("{:<22}", c), cmd_style),
                Span::styled(desc.to_string(), desc_style),
            ]));
        }
        f.render_widget(Paragraph::new(rows), chunks[palette_idx]);
    }

    let placeholder = "ask moneyball about your portfolio or type / for commands";
    let prompt_line = if app.input.is_empty() {
        // Caret sits at the input START (codex/claude-code); the dim
        // placeholder trails after it, not the other way round.
        Line::from(vec![
            Span::styled(
                "\u{276F} ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
            Span::styled(
                placeholder,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        let before = app.input[..app.cursor].to_string();
        let after = app.input[app.cursor..].to_string();
        Line::from(vec![
            Span::styled(
                "\u{276F} ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(before, Style::default().fg(Color::White)),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
            Span::styled(after, Style::default().fg(Color::White)),
        ])
    };
    f.render_widget(Paragraph::new(prompt_line), chunks[input_idx]);

    // 5. Keybinding caption (no borders) - contextual, codex-style.
    let caption_text = if palette_h > 0 {
        "  \u{2191}\u{2193} choose  \u{00B7}  \u{21B5} run  \u{00B7}  \u{21E5} complete  \u{00B7}  esc clear"
    } else {
        "  \u{21B5} send  \u{00B7}  esc clear input  \u{00B7}  \u{21E5} complete  \u{00B7}  \u{2191}\u{2193} scroll  \u{00B7}  /exit to quit"
    };
    let caption = Line::from(Span::styled(
        caption_text,
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(caption), chunks[caption_idx]);

    // 6. Status hint (when something notable happened)
    if status_h > 0 {
        if let Some(msg) = &app.status {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  {}", msg),
                    Style::default().fg(Color::Yellow),
                ))),
                chunks[status_idx],
            );
        }
    }
}

fn sep_str(width: usize) -> String {
    std::iter::repeat_n('\u{2500}', width.max(10)).collect()
}

/// Count the visual lines the chat log will render in `area.width` columns.
/// Used to size the body chunk so it doesn't expand to fill the whole
/// screen with empty rows when the chat has only a few cells.
fn count_chat_lines(app: &App, width: u16) -> usize {
    use crate::chat::ChatCell;
    let mut n = 0;
    for cell in &app.chat.cells {
        n += 1; // the blank separator line between cells
        n += cell.desired_height(width) as usize;
    }
    n
}
