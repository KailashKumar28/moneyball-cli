//! The vocabulary every front-end shares when core hands it sentences:
//! how a sentence should feel (`Tint`) and a command the user types
//! (`Command`), kept apart from the prose so a front-end without slash
//! commands can present it its own way.

use serde::Serialize;

/// How a piece of text should feel. The front-end picks the colour and
/// any glyph; core never styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tint {
    Plain,
    Good,
    Warn,
    Bad,
}

/// How a cost-per-Meta-lead ranking is introduced (the brief's products,
/// a funnel's rows), on every front-end and to the model.
pub const CHEAPEST_FIRST: &str = "cheapest Meta lead first";

/// Something the user types, and what it does, said once next to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Command {
    pub command: String,
    /// Empty when the command needs no explaining.
    pub does: String,
}

impl Command {
    pub fn new(command: &str, does: &str) -> Self {
        Command {
            command: command.into(),
            does: does.into(),
        }
    }

    /// A fix a run step or a connection names, with what it does.
    pub fn fix(fix: &str) -> Self {
        Command::new(fix, fix_does(fix))
    }
}

/// What a fix command does. Only a shell command is "in a shell": a
/// slash command runs inside moneyball.
pub fn fix_does(fix: &str) -> &'static str {
    match fix {
        "/setup" => "sets this client up: Meta, its products, goals and the model",
        "/setup meta" => "paste a fresh Meta token; products and settings stay",
        "/connect crm" => "pick your CRM and give its key; a test pull checks it",
        f if f.contains("crm secret") => "in a shell; it asks for the new token",
        f if f.contains("crm connect") => "in a shell; it walks you through connecting your CRM",
        f if f.starts_with('/') => "",
        _ => "in a shell",
    }
}

#[cfg(test)]
mod tests {
    use super::fix_does;

    #[test]
    fn only_a_shell_command_is_said_to_run_in_a_shell() {
        for slash in ["/setup", "/setup meta", "/connect crm", "/refresh"] {
            assert!(!fix_does(slash).contains("in a shell"), "{}", slash);
        }
        assert_eq!(fix_does("moneyball refresh"), "in a shell");
    }
}
