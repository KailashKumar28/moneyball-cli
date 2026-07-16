//! Minimal markdown -> styled Lines for assistant text, the way codex /
//! Claude Code render model output in a terminal: markers are consumed
//! and become styling, never shown raw. Scope is what advisor replies
//! actually contain - **bold**, `code`, bullet lists, # headers. Tables,
//! links and nesting stay plain text (models are prompted for prose).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// One styled run of text within a logical line.
#[derive(Debug, Clone, PartialEq)]
struct Run {
    text: String,
    style: Style,
}

fn base() -> Style {
    Style::default().fg(Color::White)
}
fn bold() -> Style {
    base().add_modifier(Modifier::BOLD)
}
fn code() -> Style {
    Style::default().fg(Color::Cyan)
}
fn bullet() -> Style {
    Style::default().fg(Color::Cyan)
}

/// Render markdown-ish text into wrapped, styled lines, each prefixed
/// with `indent`. `width` is the full panel width in cells.
pub(crate) fn render(text: &str, width: u16, indent: &str) -> Vec<Line<'static>> {
    let wrap_w = (width as usize).saturating_sub(indent.len()).max(20);
    let mut out = Vec::new();
    for logical in text.split('\n') {
        for visual in line_to_lines(logical, wrap_w) {
            let mut spans = vec![Span::raw(indent.to_string())];
            spans.extend(visual.into_iter().map(|r| Span::styled(r.text, r.style)));
            out.push(Line::from(spans));
        }
    }
    // Trim trailing blank lines (streaming often ends with newlines).
    while out
        .last()
        .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        && out.len() > 1
    {
        out.pop();
    }
    out
}

/// One logical markdown line -> wrapped visual lines of styled runs.
fn line_to_lines(line: &str, width: usize) -> Vec<Vec<Run>> {
    let (prefix, rest, forced_style) = block_prefix(line);
    let mut runs = inline_runs(rest);
    if let Some(s) = forced_style {
        for r in &mut runs {
            r.style = s;
        }
    }
    wrap_runs(&prefix, runs, width)
}

/// Block-level handling: bullets keep a hanging indent; headers drop the
/// hashes and render the whole line bold.
fn block_prefix(line: &str) -> (Vec<Run>, &str, Option<Style>) {
    let trimmed = line.trim_start();
    let lead = line.len() - trimmed.len();
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        let pre = vec![Run {
            text: format!("{}- ", " ".repeat(lead)),
            style: bullet(),
        }];
        return (pre, rest, None);
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=4).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
        return (vec![], trimmed[hashes + 1..].trim_start(), Some(bold()));
    }
    (vec![], line, None)
}

/// Inline `**bold**` and `` `code` `` toggles; markers are consumed.
/// An unclosed marker renders literally (never eat user text).
fn inline_runs(text: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut cur = String::new();
    let mut rest = text;
    let flush = |runs: &mut Vec<Run>, cur: &mut String| {
        if !cur.is_empty() {
            runs.push(Run {
                text: std::mem::take(cur),
                style: base(),
            });
        }
    };
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("**") {
            if let Some(end) = tail.find("**") {
                flush(&mut runs, &mut cur);
                runs.push(Run {
                    text: tail[..end].to_string(),
                    style: bold(),
                });
                rest = &tail[end + 2..];
                continue;
            }
        }
        if let Some(tail) = rest.strip_prefix('`') {
            if let Some(end) = tail.find('`') {
                flush(&mut runs, &mut cur);
                runs.push(Run {
                    text: tail[..end].to_string(),
                    style: code(),
                });
                rest = &tail[end + 1..];
                continue;
            }
        }
        let mut chars = rest.chars();
        cur.push(chars.next().expect("rest is non-empty"));
        rest = chars.as_str();
    }
    flush(&mut runs, &mut cur);
    runs
}

/// Greedy word-wrap over styled runs. `prefix` starts the first visual
/// line; continuation lines get a same-width hanging indent so bullet
/// text stays aligned.
fn wrap_runs(prefix: &[Run], runs: Vec<Run>, width: usize) -> Vec<Vec<Run>> {
    let prefix_w: usize = prefix.iter().map(|r| r.text.chars().count()).sum();
    let hang = " ".repeat(prefix_w);
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut cur: Vec<Run> = prefix.to_vec();
    let mut cur_w = prefix_w;
    for run in runs {
        for word in split_keep_spaces(&run.text) {
            let w = word.chars().count();
            if cur_w + w > width && cur_w > prefix_w {
                lines.push(std::mem::take(&mut cur));
                if !hang.is_empty() {
                    cur.push(Run {
                        text: hang.clone(),
                        style: base(),
                    });
                }
                cur_w = prefix_w;
                if word.trim().is_empty() {
                    continue; // no leading space on a wrapped line
                }
            }
            cur.push(Run {
                text: word.to_string(),
                style: run.style,
            });
            cur_w += w;
        }
    }
    lines.push(cur);
    lines
}

/// Split into words and the single spaces between them, preserving both
/// so styled runs rejoin without losing spacing.
fn split_keep_spaces(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = None::<bool>;
    for (i, c) in s.char_indices() {
        let is_space = c == ' ';
        if in_space.is_some_and(|prev| prev != is_space) {
            out.push(&s[start..i]);
            start = i;
        }
        in_space = Some(is_space);
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn bold_and_code_markers_are_consumed() {
        let lines = render("scale **Lead Gen** via `/funnel`", 80, "");
        let text = flat(&lines);
        assert!(!text.contains("**") && !text.contains('`'), "{}", text);
        assert!(text.contains("Lead Gen") && text.contains("/funnel"));
        // The bold words carry the BOLD modifier (wrap splits runs by word).
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.content == "Lead" && s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn unclosed_markers_render_literally() {
        assert!(flat(&render("a ** b", 80, "")).contains("**"));
        assert!(flat(&render("a ` b", 80, "")).contains('`'));
    }

    #[test]
    fn bullets_get_hanging_indent_on_wrap() {
        let lines = render(
            "- **Portfolio** overview across products with long text that wraps",
            30,
            "",
        );
        let text = flat(&lines);
        let mut it = text.lines();
        assert!(it.next().unwrap().starts_with("- "));
        assert!(it.next().unwrap().starts_with("  "));
        assert!(!text.contains("**"));
    }

    #[test]
    fn headers_drop_hashes_and_bold() {
        let lines = render("## Key numbers", 80, "");
        let text = flat(&lines);
        assert_eq!(text.trim(), "Key numbers");
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn plain_text_unchanged_and_wrapped() {
        let lines = render("word ".repeat(20).trim(), 30, "  ");
        assert!(lines.len() > 1);
        assert!(lines
            .iter()
            .all(|l| flat(std::slice::from_ref(l)).starts_with("  ")));
    }
}
