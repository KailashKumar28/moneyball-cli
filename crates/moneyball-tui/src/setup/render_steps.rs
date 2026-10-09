//! Wizard step renders: indicator, collapsed summaries, and the
//! per-step body views.

use super::*;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

pub(crate) fn render_setup(f: &mut ratatui::Frame, area: Rect, s: &SetupState) {
    // Codex-style vertical stack (no boxed modal):
    //   step indicator strip  (1 row)
    //   completed-step lines  (1 row each)
    //   active-step panel     (variable, plain text + rounded input border)
    //   error/footer hint     (1-2 rows, single dim line)
    //
    // The previous boxed-modal layout clipped the input prompt on step 2
    // (products) and step 3 (goals) once the user had >= 3 products.
    // Composing manually lets each section size to its actual content.
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.extend(render_step_indicator(s));
    lines.push(Line::from(""));
    let done = render_completed_steps(s);
    if !done.is_empty() {
        lines.extend(done);
        lines.push(Line::from(""));
    }
    lines.extend(render_active_step(s));

    // The error wraps under its own text ("  ! " hangs), every line of it
    // shown above the key hints.
    let width = area.width.saturating_sub(4) as usize;
    let error: Vec<String> = s
        .error
        .as_deref()
        .map(|e| moneyball_core::text::wrap_columns(e, width, width))
        .unwrap_or_default();
    let hint_h = error.len() as u16 + 1;
    let content_h = area.height.saturating_sub(hint_h).max(3);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(content_h), Constraint::Length(hint_h)])
        .split(area);

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), chunks[0]);

    let mut footer_lines: Vec<Line> = error
        .iter()
        .enumerate()
        .map(|(i, l)| {
            Line::from(Span::styled(
                format!("  {} {}", if i == 0 { "!" } else { " " }, l),
                Style::default().fg(Color::Red),
            ))
        })
        .collect();
    footer_lines.push(Line::from(Span::styled(
        "  enter next  \u{00B7}  esc exit setup  \u{00B7}  ctrl+c quit",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(footer_lines).wrap(Wrap { trim: false }),
        chunks[1],
    );
}

/// Top progress strip: `1 \u{00B7} workspace   2 \u{00B7} token   ...` with the current step highlighted.
/// Labels match the collapsed-step summaries in `render_completed_steps`.
fn render_step_indicator(s: &SetupState) -> Vec<Line<'static>> {
    let total = 5;
    let cur = s.step.min(total - 1);
    let labels = ["workspace", "token", "products", "goals", "model"];
    let mut spans: Vec<Span<'static>> = vec![Span::styled("  ", Style::default())];
    for (i, label) in labels.iter().enumerate() {
        let is_current = i == cur;
        let style = if is_current {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let marker = if is_current { "\u{25B8}" } else { " " };
        spans.push(Span::styled(
            format!("{} {} \u{00B7} {}", marker, i + 1, label),
            style,
        ));
        if i + 1 < total {
            spans.push(Span::styled("   ", Style::default().fg(Color::DarkGray)));
        }
    }
    vec![Line::from(spans)]
}

/// One-line summaries for completed steps (workspace / token / accounts / products / goals).
fn render_completed_steps(s: &SetupState) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    if s.step >= 1 {
        // step 0 (workspace) is completed. Tail-truncate long paths so the
        // one-line summary never wraps (same treatment as the input box).
        const VISIBLE: usize = 56;
        let n = s.workspace_path.chars().count();
        let shown: String = if n > VISIBLE {
            let tail: String = s.workspace_path.chars().skip(n - (VISIBLE - 1)).collect();
            format!("\u{2026}{}", tail)
        } else {
            s.workspace_path.clone()
        };
        out.push(Line::from(Span::styled(
            done_label(1, "workspace", &shown),
            Style::default().fg(Color::Green),
        )));
    }
    if s.step >= 2 && s.meta_connected {
        let bullets = "\u{2022}".repeat(s.meta_token_len.min(10));
        let n = s.meta_token_len;
        out.push(Line::from(Span::styled(
            done_label(2, "meta token", &format!("{} ({} chars)", bullets, n)),
            Style::default().fg(Color::Green),
        )));
    }
    if s.step >= 3 {
        let n = s.products.len();
        out.push(Line::from(Span::styled(
            done_label(3, "products", &format!("{} configured", n)),
            Style::default().fg(Color::Green),
        )));
    }
    if !s.llm_provider_id.is_empty() && !s.llm_model.is_empty() && s.llm_key_len > 0 {
        let provider = s.llm_provider_id.as_str();
        let model = s.llm_model.clone();
        let n = s.llm_key_len;
        let bullets = "\u{2022}".repeat(n.min(10));
        out.push(Line::from(Span::styled(
            done_label(
                5,
                "model",
                &format!("{} \u{00B7} {} ({})", provider, model, bullets),
            ),
            Style::default().fg(Color::Green),
        )));
    }
    out
}

/// "✓ n · label   value", every value in one column.
fn done_label(n: usize, label: &str, value: &str) -> String {
    format!("  \u{2713} {} \u{00B7} {:<12}{}", n, label, value)
}

/// Active step's content as plain lines. Each step helper returns a `Vec<Line>`
/// so we don't pay the cost of a `Paragraph` block just to compose it.
fn render_active_step(s: &SetupState) -> Vec<Line<'static>> {
    match s.step {
        0 => render_step_workspace(s),
        1 => render_step_meta(s),
        2 => render_step_products(s),
        3 => render_step_goals(s),
        4 => super::render_llm::render_step_llm(s),
        _ => vec![Line::from("done")],
    }
}

pub(super) fn styled_title(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {}", text),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Rounded cyan border opener for the active input field. The interior
/// width is hardcoded to 60 chars; the closing border uses the same
/// constant so they line up visually. Title appears on the top border
/// (e.g. "╭ workspace path ────╮").
pub(super) const INPUT_BOX_INNER: usize = 60;

pub(super) fn input_box_open(title: &str) -> Line<'static> {
    // "  ╭ title ───╮" is as wide as the close "  ╰─────╯": 2 + 1 + 1 +
    // title + 1 + fill + 1 = 2 + 1 + INPUT_BOX_INNER + 1.
    let fill = INPUT_BOX_INNER.saturating_sub(title.chars().count() + 2);
    Line::from(Span::styled(
        format!("  \u{256D} {} {}\u{256E}", title, "\u{2500}".repeat(fill)),
        Style::default().fg(Color::Cyan),
    ))
}

/// Closing border line for the active input field. Width matches
/// `input_box_open` so the corners line up.
pub(super) fn input_box_close() -> Line<'static> {
    Line::from(Span::styled(
        format!("  \u{2570}{}\u{256F}", "\u{2500}".repeat(INPUT_BOX_INNER)),
        Style::default().fg(Color::Cyan),
    ))
}

/// The row inside the box: "> value", the cursor when it is the active
/// field, and the right border. A value longer than the box shows its
/// tail - you edit the end.
pub(super) fn input_row(value: &str, active: bool) -> Line<'static> {
    // The interior less "  > ", the cursor and the space before the border.
    const ROOM: usize = INPUT_BOX_INNER - 6;
    let n = value.chars().count();
    let shown: String = if n > ROOM {
        std::iter::once('\u{2026}')
            .chain(value.chars().skip(n - (ROOM - 1)))
            .collect()
    } else {
        value.to_string()
    };
    let border = Style::default().fg(Color::Cyan);
    let text = if active {
        border.add_modifier(Modifier::BOLD)
    } else {
        border
    };
    Line::from(vec![
        Span::styled("  \u{2502}", border),
        Span::styled(
            format!(
                "  > {}{}{} ",
                shown,
                if active { "\u{2588}" } else { " " },
                " ".repeat(ROOM - shown.chars().count())
            ),
            text,
        ),
        Span::styled("\u{2502}", border),
    ])
}

fn render_step_workspace(s: &SetupState) -> Vec<Line<'static>> {
    let mut lines = vec![
        styled_title("Workspace path"),
        Line::from(""),
        Line::from("  This is where moneyball will read snapshots + write ledger/runs."),
        Line::from("  The directory will be auto-created if it does not exist."),
        Line::from(""),
    ];
    // Rounded cyan border around the input field (codex auth.rs pattern).
    lines.push(input_box_open("workspace path"));
    lines.push(input_row(&s.workspace_path, true));
    lines.push(input_box_close());
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  enter to accept \u{00B7} backspace to edit \u{00B7} esc clears input",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn render_step_meta(s: &SetupState) -> Vec<Line<'static>> {
    match s.meta_substep {
        0 => {
            let n = s.meta_input.chars().count();
            let mut lines = vec![
                styled_title("Meta API access token"),
                Line::from(""),
                Line::from("  Paste a long-lived Meta Marketing API access token"),
                Line::from("  (the one with ads_read permission; get one at"),
                Line::from("  developers.facebook.com -> Tools -> Marketing API)."),
                Line::from(""),
            ];
            // Rounded cyan border around the token input.
            lines.push(input_box_open("meta token"));
            lines.push(input_row(&"\u{2022}".repeat(n), true));
            lines.push(input_box_close());
            lines.push(Line::from(Span::styled(
                format!("  ({} chars)", n),
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Token is saved to ~/.moneyball/auth.json (user-only permissions).",
                Style::default().fg(Color::DarkGray),
            )));
            lines
        }
        1 => {
            // Multi-select list with scroll. Visible rows must match VISIBLE_ROWS
            // in handle_select_keys.
            const VISIBLE_ROWS: usize = 12;
            let n = s.meta_discovered.len();
            let selected = s.meta_selections.iter().filter(|&&b| b).count();
            let mut lines = vec![
                styled_title(&format!("Select ad accounts ({} of {} chosen)",
                    selected, n)),
                Line::from(""),
                Line::from(Span::styled(
                    "  \u{2191}\u{2193}/PgUp/PgDn move  Space=toggle  a=all/none  Enter=confirm  Esc=back",
                    Style::default().fg(Color::DarkGray))),
                Line::from(""),
            ];
            let end = (s.meta_scroll + VISIBLE_ROWS).min(n);
            let start = s.meta_scroll.min(end);
            for i in start..end {
                let a = &s.meta_discovered[i];
                let status: &'static str = match a.account_status {
                    Some(1) => "ACTIVE",
                    Some(2) => "DISABLED",
                    Some(3) => "UNSETTLED",
                    Some(9) => "PENDING_RISK_REVIEW",
                    Some(101) => "PENDING_SETUP",
                    Some(_) => "OTHER",
                    None => "?",
                };
                let checkbox = if s.meta_selections[i] { "[x]" } else { "[ ]" };
                let marker = if i == s.meta_highlight {
                    "\u{25B8}"
                } else {
                    " "
                };
                let text = format!(
                    "  {} {} [{:>2}] {} - {} ({})",
                    marker,
                    checkbox,
                    i + 1,
                    a.id,
                    a.name,
                    status
                );
                let style = if i == s.meta_highlight {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if s.meta_selections[i] {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
            if end < n {
                lines.push(Line::from(Span::styled(
                    format!("  ... {} more below (PgDn to scroll)", n - end),
                    Style::default().fg(Color::DarkGray),
                )));
            } else if start > 0 {
                lines.push(Line::from(Span::styled(
                    "  ... PgUp to scroll up",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            lines
        }
        2 => {
            let mut lines = vec![
                styled_title("Name your products (optional)"),
                Line::from(""),
                Line::from("  Defaults: each product uses the account's display name."),
                Line::from("  To rename, type e.g.  1=BrandName 3=OtherName"),
                Line::from(Span::styled(
                    "  Press Enter on blank line to keep defaults.",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(""),
            ];
            for (i, &idx) in s.meta_selected.iter().enumerate() {
                let a = &s.meta_discovered[idx];
                lines.push(Line::from(format!(
                    "  [{}] {} (default: {})",
                    i + 1,
                    a.id,
                    a.name
                )));
            }
            lines.push(Line::from(""));
            lines.push(input_box_open("rename"));
            lines.push(input_row(&s.meta_rename_input, true));
            lines.push(input_box_close());
            lines
        }
        _ => vec![Line::from("done")],
    }
}

fn render_step_products(s: &SetupState) -> Vec<Line<'static>> {
    // Step 3 is now mostly empty because token is mandatory; if meta
    // succeeded, products were auto-populated. We just confirm here.
    // The input box is hidden (read-only confirmation) and the Enter key
    // proceeds to step 4 (goals).
    let mut lines = vec![styled_title("Confirm products"), Line::from("")];
    if s.products.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no products yet)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "  {} product{} configured:",
                s.products.len(),
                if s.products.len() == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::Green),
        )));
        for (n, a) in &s.products {
            lines.push(Line::from(Span::styled(
                format!("    \u{2713} {}  \u{2192}  {}", n, a),
                Style::default(),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  press enter to continue \u{00B7} esc to go back",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn render_step_goals(s: &SetupState) -> Vec<Line<'static>> {
    let mut lines = vec![
        styled_title("Goals per product"),
        Line::from(""),
        Line::from("  Format: ProdName=Number, space- or comma-separated. Multi-word"),
        Line::from("  product names are fine: the parser reads up to the '='."),
        Line::from("  Example: Namma Mane=10 Valmark CityVille=15"),
        Line::from(Span::styled(
            "  Press Enter on blank line to accept all defaults.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    for (n, _) in &s.products {
        lines.push(Line::from(Span::styled(
            format!("    {} = 10 (default)", n),
            Style::default(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(input_box_open("goals"));
    lines.push(input_row(&s.goals_input, true));
    lines.push(input_box_close());
    lines
}
