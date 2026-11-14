//! A core `Command` as this app offers it. Core words every next step as
//! something the user types (`/refresh`, `/connect crm`, `moneyball crm
//! secret x`); the app has no slash commands, so a refresh and a report
//! become its own buttons and everything else a terminal command to copy.
//! This is the only place that translation happens: `translate` for the
//! screen, and `say`, its twin in words, for what the advisor reads.

use moneyball_core::words::Command;
use serde::Serialize;

/// What the interface shows for a next step. `Terminal` is never run by
/// the app: it is shown as "In Terminal" with a copy button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// A button that starts a refresh; `force` pulls Meta again.
    Refresh {
        label: String,
        does: String,
        force: bool,
    },
    /// A button that generates the day's report.
    Report {
        label: String,
        does: String,
    },
    Terminal {
        command: String,
        does: String,
    },
}

/// The terminal program every shell fix starts with.
const CLI: &str = "moneyball";

pub fn translate(c: &Command) -> Action {
    let does = c.does.clone();
    match c.command.as_str() {
        "/refresh" => Action::Refresh {
            label: "Refresh".into(),
            does,
            force: false,
        },
        "/refresh --force" => Action::Refresh {
            label: "Pull Meta again".into(),
            does,
            force: true,
        },
        "/report" => Action::Report {
            label: "Generate report".into(),
            does,
        },
        "/connect crm" => terminal("moneyball connect crm", &does),
        "/schedule" => terminal("moneyball schedule --at 07:30", &does),
        other => match (other.strip_prefix("/ledger"), other.starts_with('/')) {
            // "/ledger <id> applied" has a shell twin of the same shape.
            (Some(rest), _) => terminal(&format!("moneyball ledger{}", rest), &does),
            (None, true) => inside_moneyball(other, &does),
            (None, false) => shell(other, &does),
        },
    }
}

/// `translate` in words, for the advisor (`FrontEnd::say`): how this
/// app's user does a step core names as the terminal app types it. Never
/// returns a slash command - the model would hand it to the user, and
/// nothing here takes one.
pub fn say(step: &str) -> String {
    let step = step.trim();
    let in_terminal = |command: &str| format!("`{}` in Terminal", command);
    match translate(&Command::new(step, "")) {
        // Cmd-R starts one from any screen; not every screen has the button.
        Action::Refresh { force: false, .. } => "Refresh (Cmd-R)".into(),
        Action::Refresh { force: true, .. } => {
            "the \"Pull Meta again\" button on the Today screen".into()
        }
        Action::Report { .. } => "the Generate button on the Report screen".into(),
        // The brief and the funnel are screens here, not commands.
        Action::Terminal { .. } if step == "/brief" => "the Brief screen".into(),
        Action::Terminal { .. } if step.starts_with("/funnel") => "the Funnel screen".into(),
        // No shell twin: the step is done inside the terminal app.
        Action::Terminal { command, .. } if step.starts_with('/') && command == CLI => {
            match inside(step) {
                Some(place) => format!("{}, then {}", in_terminal(CLI), place),
                None => in_terminal(CLI),
            }
        }
        Action::Terminal { command, .. } => in_terminal(&command),
    }
}

fn terminal(command: &str, does: &str) -> Action {
    Action::Terminal {
        command: command.into(),
        does: does.into(),
    }
}

/// Where a slash command with no shell twin is done inside the terminal
/// app, in words. Never the command itself: nothing this app shows or
/// says may read as something to type (AGENTS.md).
fn inside(slash: &str) -> Option<&'static str> {
    match slash {
        "/setup" => Some("its setup"),
        "/setup meta" => Some("its Meta setup"),
        _ => None,
    }
}

/// A slash command with no shell twin (`/setup`, `/setup meta`): the
/// terminal app is where it runs, so the command to paste opens that,
/// and `does` says where to go in it, the way `say` does.
fn inside_moneyball(slash: &str, does: &str) -> Action {
    let does = match (inside(slash), not_in_a_shell(does)) {
        (Some(place), "") => format!("then open {}", place),
        (Some(place), d) => format!("then open {} - {}", place, d),
        (None, d) => d.to_string(),
    };
    terminal(CLI, &does)
}

/// Core's "in a shell" dropped from what a command does: "In Terminal"
/// already says where it runs.
fn not_in_a_shell(does: &str) -> &str {
    does.strip_prefix("in a shell")
        .map_or(does, |rest| rest.trim_start_matches("; "))
}

/// A shell command from core, made pasteable: several joined with "and"
/// chain with `&&`, and a trailing "(note)" moves into `does`.
fn shell(command: &str, does: &str) -> Action {
    let (command, note) = match command.split_once(" (") {
        Some((cmd, note)) => (cmd, note.trim_end_matches(')')),
        None => (command, ""),
    };
    let command = command.replace(" and moneyball ", " && moneyball ");
    let does = match (note, not_in_a_shell(does)) {
        ("", d) => d.to_string(),
        (n, "") => n.to_string(),
        (n, d) => format!("{}; {}", n, d),
    };
    terminal(&command, &does)
}

#[cfg(test)]
#[path = "action_tests.rs"]
pub(crate) mod tests;
