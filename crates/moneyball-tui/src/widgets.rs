//! Shared visual elements: logo, context bar, status glyphs.
//!
//! All views (setup wizard, brief view) render the same logo + context bar
//! at the top so the user always sees where they are and what state the
//! workspace is in. The body differs per view; the chrome stays.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// Status glyphs
pub const BRAND: &str = "\u{25C6}"; // black diamond
pub const OK: &str = "\u{25CF}"; // black circle
pub const LOAD: &str = "\u{25D0}"; // circle with left half black
pub const IDLE: &str = "\u{25CB}"; // white circle

// Status kinds drive the color of the trailing status indicator.
#[derive(Debug, Clone, Copy)]
pub enum Status {
    Ready,
    Loading,
    NoData,
    Error,
    /// Setup wizard in progress - workspace not yet configured.
    Idle,
}

impl Status {
    pub fn glyph(self) -> &'static str {
        match self {
            Status::Ready => OK,
            Status::Loading => LOAD,
            Status::NoData => IDLE,
            Status::Error => IDLE,
            Status::Idle => IDLE,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Ready => "ready",
            Status::Loading => "loading",
            Status::NoData => "no data",
            Status::Error => "error",
            Status::Idle => "first-run setup",
        }
    }
    pub fn color(self) -> Color {
        match self {
            Status::Ready => Color::Green,
            Status::Loading => Color::Yellow,
            Status::NoData => Color::DarkGray,
            Status::Error => Color::Red,
            Status::Idle => Color::DarkGray,
        }
    }
}

/// Top-of-screen logo. Two visual bands: brand mark + tagline.
pub fn logo() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                BRAND,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "MONEYBALL",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  \u{00B7}  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "read-only Meta-ads advisor",
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(""), // spacer line so the logo doesn't crowd the context bar
    ]
}

/// Status / context line under the logo. Always reflects live state.
pub fn context_line(workspace: &str, snapshot: Option<&str>, status: Status) -> Line<'static> {
    let snap_part = match snapshot {
        Some(d) => format!("  \u{00B7}  snapshot: {}", d),
        None => "  \u{00B7}  no snapshot".to_string(),
    };
    Line::from(vec![
        Span::styled(
            "  \u{2514}\u{2500} workspace: ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(workspace.to_string(), Style::default().fg(Color::Gray)),
        Span::styled(snap_part, Style::default().fg(Color::DarkGray)),
        Span::styled("  \u{00B7}  ", Style::default().fg(Color::DarkGray)),
        Span::styled(status.glyph(), Style::default().fg(status.color())),
        Span::styled(" ", Style::default().fg(Color::DarkGray)),
        Span::styled(status.label(), Style::default().fg(status.color())),
    ])
}

/// Welcome screen body — shown when setup is complete but no snapshot data yet.
/// Replaces the old "blank box with a status error" empty state.
pub fn welcome_text(products: &[String]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Waiting for first snapshot",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  moneyball is configured but there's no snapshot data to read yet."),
        Line::from("  Run your Meta + CRM fetcher to populate"),
        Line::from(Span::styled(
            "    <workspace>/moneyball/history/snap/<YYYY-MM-DD>/*.json",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from("  then press /brief to reload."),
        Line::from(""),
    ];

    if !products.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "  {} product{} configured:",
                products.len(),
                if products.len() == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::Cyan),
        )));
        for p in products {
            lines.push(Line::from(format!("    \u{00B7} {}", p)));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "  Quick actions",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from("    /brief     reload from disk"));
    lines.push(Line::from("    /setup     re-run the setup wizard"));
    lines.push(Line::from("    /quit      exit moneyball"));
    lines
}
