//! The Meta token step's screens: the token paste, the account picker,
//! the names to give them. Split from render_steps.rs (size cap).

use super::render_steps::{input_box_close, input_box_open, input_row, styled_title};
use super::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub(super) fn render_step_meta(s: &SetupState) -> Vec<Line<'static>> {
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
