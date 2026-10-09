//! The model step's screens: provider list, key paste, model list, and the
//! custom provider's URL and model. Split from render_steps.rs (size cap).

use super::render_steps::{input_box_close, input_box_open, styled_title};
use super::*;
use moneyball_core::provider::{built_in_presets, models_for, ModelProviderInfo, WireApi};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Step 4: LLM provider config. Required - /brief and /ask depend on this.
///
/// Substeps:
///   0 -> pick a built-in preset (openai / anthropic / minimax / ollama)
///        or "custom"
///   1 -> paste the API key (masked + char count)
///   2 -> pick a curated model from the provider's list
///   3 -> (custom only) enter the base URL
///
/// Saves the API key to ~/.moneyball/auth.json via `secrets::store_llm_key` on
/// advance. The provider entry is persisted to config.json alongside the
/// rest of the workspace config in advance_save.
pub(super) fn render_step_llm(s: &SetupState) -> Vec<Line<'static>> {
    match s.llm_substep {
        0 => render_llm_pick_provider(s),
        1 => render_llm_paste_key(s),
        2 => render_llm_pick_model(s),
        _ => render_llm_custom(s),
    }
}

fn render_llm_pick_provider(s: &SetupState) -> Vec<Line<'static>> {
    const VISIBLE_ROWS: usize = 6;
    let presets = built_in_presets();
    let total = presets.len() + 1; // +1 for "custom"
    let mut lines = vec![
        styled_title("LLM provider"),
        Line::from(""),
        Line::from("  Pick the model provider that drives /brief and /ask."),
        Line::from("  Custom lets you point at any OpenAI/Anthropic-compatible URL."),
        Line::from(""),
        Line::from(Span::styled(
            "  \u{2191}\u{2193} move  Enter=select  Esc=back",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    let end = (s.llm_scroll + VISIBLE_ROWS).min(total);
    let start = s.llm_scroll.min(end);
    let mut rows: Vec<(String, String)> = presets
        .iter()
        .map(|(id, p)| {
            let wire = match p.wire_api {
                WireApi::Responses => "Responses",
                WireApi::ChatCompletions => "Chat",
                WireApi::Messages => "Messages",
            };
            (
                id.to_string(),
                format!("  {} - {} ({})", p.name, p.base_url, wire),
            )
        })
        .collect();
    rows.push((
        "custom".to_string(),
        "  custom - your own URL (any wire protocol)".to_string(),
    ));

    for (i, (id, text)) in rows.iter().enumerate().skip(start).take(end - start) {
        let marker = if i == s.llm_highlight {
            "\u{25B8}"
        } else {
            " "
        };
        let style = if i == s.llm_highlight {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let line = format!("  {} {}", marker, text);
        let _ = id; // silence unused warning; id is available for future row-action
        lines.push(Line::from(Span::styled(line, style)));
    }
    if end < total {
        lines.push(Line::from(Span::styled(
            format!("  ... {} more below", total - end),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn render_llm_paste_key(s: &SetupState) -> Vec<Line<'static>> {
    let n = s.llm_input.chars().count();
    let provider_label = if s.llm_provider_id == "custom" {
        format!("custom ({})", s.llm_url)
    } else {
        s.llm_provider_id.clone()
    };
    let mut lines = vec![
        styled_title("LLM API key"),
        Line::from(""),
        Line::from(format!("  Provider: {}", provider_label)),
        Line::from(""),
        Line::from("  Paste the API key for this provider."),
        Line::from("  It is saved to ~/.moneyball/auth.json (user-only permissions)"),
        Line::from("  and never written to disk in plaintext."),
        Line::from(""),
    ];
    // Cap masked at 48 bullets so the line never overflows the box's
    // 60-char interior. Anything longer than 48 shows "..." suffix.
    let masked: String = "\u{2022}".repeat(n.min(48));
    let suffix: String = if n > 48 { "+".into() } else { String::new() };
    lines.push(input_box_open("api key"));
    lines.push(Line::from(Span::styled(
        format!("  \u{2502}  > {}{}\u{2588}", masked, suffix),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(input_box_close());
    lines.push(Line::from(Span::styled(
        format!("  ({} chars)", n),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  enter to validate + continue  \u{00B7}  esc exit setup",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn render_llm_pick_model(s: &SetupState) -> Vec<Line<'static>> {
    let preset = if s.llm_provider_id == "custom" {
        ModelProviderInfo {
            name: "custom".into(),
            base_url: s.llm_url.clone(),
            ..Default::default()
        }
    } else {
        s.llm_provider
            .clone()
            .unwrap_or_else(ModelProviderInfo::openai)
    };
    let models = models_for(&preset);
    let total = models.len();
    let mut lines = vec![
        styled_title("Model"),
        Line::from(""),
        Line::from(format!("  Provider: {}", s.llm_provider_id)),
        Line::from(""),
        Line::from("  Pick the model. /brief and /ask will use it."),
        Line::from(Span::styled(
            "  \u{2191}\u{2193} move  Enter=select  Esc=back",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    for (i, m) in models.iter().enumerate() {
        let marker = if i == s.llm_highlight {
            "\u{25B8}"
        } else {
            " "
        };
        let style = if i == s.llm_highlight {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("  {} {}", marker, m),
            style,
        )));
    }
    if total == 1 && models[0] == "custom" {
        // For custom provider, ask for the model slug via free text.
        lines.push(Line::from(""));
        lines.push(input_box_open("model"));
        lines.push(Line::from(Span::styled(
            format!("  \u{2502}  > {}\u{2588}", s.llm_model),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(input_box_close());
        let _ = total; // silence unused
    }
    lines
}

/// Custom provider: its base URL, then its model, each in its own box
/// (this substep used to draw the provider list while keys typed blind).
fn render_llm_custom(s: &SetupState) -> Vec<Line<'static>> {
    let field = |title: &str, value: &str, active: bool| {
        vec![
            input_box_open(title),
            Line::from(Span::styled(
                format!(
                    "  \u{2502}  > {}{}",
                    value,
                    if active { "\u{2588}" } else { "" }
                ),
                Style::default().fg(Color::Cyan).add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            )),
            input_box_close(),
        ]
    };
    let mut lines = vec![
        styled_title("Custom provider"),
        Line::from(""),
        Line::from("  The base URL of any OpenAI- or Anthropic-compatible API,"),
        Line::from("  e.g. http://localhost:11434/v1 - then the model to call there."),
        Line::from(""),
    ];
    lines.extend(field("base URL", &s.llm_url, !s.llm_url_entered));
    if s.llm_url_entered {
        lines.push(Line::from(""));
        lines.extend(field("model", &s.llm_model, true));
    }
    lines
}
