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

/// What a fix command does.
pub fn fix_does(fix: &str) -> &'static str {
    if fix == "/setup meta" {
        "paste a fresh Meta token; products and settings stay"
    } else if fix.contains("crm secret") {
        "in a shell; it asks for the new token"
    } else if fix == "/connect crm" {
        "pick your CRM and give its key; a test pull checks it"
    } else if fix.contains("crm connect") {
        "in a shell; it walks you through connecting your CRM"
    } else {
        "in a shell"
    }
}
