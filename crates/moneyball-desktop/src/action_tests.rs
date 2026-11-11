//! Every fix string core can produce (grep `fix` and `Command::new` in
//! moneyball-core) goes through `translate`, and no terminal command it
//! emits names a subcommand the binary does not have.

use super::*;

/// The command strings core produces, with where each comes from.
const CORE_COMMANDS: &[&str] = &[
    // landing/story.rs, landing/day.rs, refresh/finish.rs, connections
    "/refresh",
    "/refresh --force",
    // refresh/finish.rs
    "/report",
    // landing/mod.rs
    "/schedule",
    // error.rs (MetaAuth), connections/mod.rs
    "/setup meta",
    "/setup",
    // refresh/mod.rs, landing/story.rs, connections/mod.rs
    "/connect crm",
    // connections/mod.rs, crm/fetch.rs status_error
    "moneyball crm secret leadzump_token",
    "moneyball crm secret a and moneyball crm secret b",
    "moneyball crm secret <name> (the name crm.toml references as secret:<name>)",
];

/// Steps core passes to `say` beyond the fixes above (agent/prompt.rs,
/// agent/exec.rs, diagnose/print.rs), and one it does not know.
const SAID_ONLY: &[&str] = &[
    "/brief",
    "/funnel Namma Mane",
    "/ledger mb-20261022-1 applied",
    "moneyball refresh",
    "/no-such-command now",
];

/// A word that reads as a slash command: "/refresh", "(/setup", "`/x`".
pub(crate) fn slash_command(text: &str) -> Option<&str> {
    text.split_whitespace().find(|w| {
        let w = w.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '/');
        let mut chars = w.chars();
        chars.next() == Some('/') && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
    })
}

#[test]
fn say_is_translate_in_words_and_never_a_slash_command() {
    assert_eq!(say("/refresh"), "the Refresh button");
    assert_eq!(say("/report"), "the Generate button on the Report screen");
    assert_eq!(say("/brief"), "the Brief screen");
    assert_eq!(say("/connect crm"), "`moneyball connect crm` in Terminal");
    assert_eq!(say("/setup"), "`moneyball` in Terminal, then its setup");
    assert_eq!(
        say("/ledger mb-20261022-1 applied"),
        "`moneyball ledger mb-20261022-1 applied` in Terminal"
    );
    assert_eq!(
        say("moneyball crm secret a and moneyball crm secret b"),
        "`moneyball crm secret a && moneyball crm secret b` in Terminal"
    );
    assert_eq!(say("/no-such-command now"), "`moneyball` in Terminal");
    for step in CORE_COMMANDS.iter().chain(SAID_ONLY) {
        let said = say(step);
        assert_eq!(slash_command(&said), None, "{} -> {}", step, said);
        assert_ne!(&said, step, "{} is not typed here as it is", step);
    }
    assert_eq!(slash_command("run (/setup) first"), Some("(/setup)"));
    assert_eq!(slash_command("Rs/qualified and 4/day at a/b"), None);
}

fn act(command: &str) -> Action {
    translate(&Command::fix(command))
}

fn terminal_of(a: &Action) -> Option<(&str, &str)> {
    match a {
        Action::Terminal { command, does } => Some((command, does)),
        _ => None,
    }
}

#[test]
fn refresh_and_report_become_buttons() {
    let refresh = translate(&Command::new("/refresh", "runs the rest"));
    assert_eq!(
        refresh,
        Action::Refresh {
            label: "Refresh".into(),
            does: "runs the rest".into(),
            force: false
        }
    );
    let forced = translate(&Command::new("/refresh --force", "pulls Meta again"));
    assert!(matches!(forced, Action::Refresh { force: true, .. }));
    let report = translate(&Command::new("/report", "builds one from it"));
    assert_eq!(
        report,
        Action::Report {
            label: "Generate report".into(),
            does: "builds one from it".into()
        }
    );
}

#[test]
fn every_other_command_is_a_shell_command_to_paste() {
    assert_eq!(
        terminal_of(&act("/connect crm")),
        Some((
            "moneyball connect crm",
            "pick your CRM and give its key; a test pull checks it"
        ))
    );
    assert_eq!(
        terminal_of(&act("/setup meta")),
        Some((
            "moneyball",
            "then type /setup meta there - paste a fresh Meta token; products and settings stay"
        ))
    );
    assert_eq!(
        terminal_of(&act("moneyball crm secret leadzump_token")),
        Some((
            "moneyball crm secret leadzump_token",
            "it asks for the new token"
        ))
    );
    assert_eq!(
        terminal_of(&act("moneyball crm secret a and moneyball crm secret b")).map(|t| t.0),
        Some("moneyball crm secret a && moneyball crm secret b")
    );
    assert_eq!(
        terminal_of(&act(CORE_COMMANDS[9])),
        Some((
            "moneyball crm secret <name>",
            "the name crm.toml references as secret:<name>; it asks for the new token"
        ))
    );
}

#[test]
fn no_slash_command_reaches_the_command_line_and_json_is_tagged_by_kind() {
    for c in CORE_COMMANDS {
        let action = act(c);
        if let Some((command, _)) = terminal_of(&action) {
            assert!(!command.contains('/'), "{} -> {}", c, command);
            assert!(command.starts_with(CLI), "{} -> {}", c, command);
        }
        let json = serde_json::to_value(&action).unwrap();
        let kind = json["kind"].as_str().unwrap();
        assert!(
            ["refresh", "report", "terminal"].contains(&kind),
            "{}",
            json
        );
    }
}

/// Each `moneyball <sub> [<sub>]` we emit is a real clap variant in the
/// binary's source: a renamed subcommand fails here, not in a user's
/// terminal.
#[test]
fn terminal_commands_exist_in_the_binary() {
    let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../moneyball/src");
    let read = |f: &str| std::fs::read_to_string(bin.join(f)).unwrap();
    let (main, connect, crm) = (read("main.rs"), read("connect_cli.rs"), read("crm_cli.rs"));
    let has = |src: &str, word: &str| {
        let mut chars = word.chars();
        let first = chars.next().map(|c| c.to_ascii_uppercase());
        let name: String = first.into_iter().chain(chars).collect();
        [" {", ",", "("]
            .iter()
            .any(|end| src.contains(&format!("\n    {}{}", name, end)))
    };
    for c in CORE_COMMANDS.iter().chain(SAID_ONLY) {
        let action = act(c);
        let Some((command, _)) = terminal_of(&action) else {
            continue;
        };
        for one in command.split(" && ") {
            let words: Vec<&str> = one.split(' ').collect();
            match words.as_slice() {
                ["moneyball"] => {}
                ["moneyball", "refresh"] => assert!(has(&main, "refresh"), "{}", one),
                ["moneyball", "connect", sub] => {
                    assert!(has(&main, "connect") && has(&connect, sub), "{}", one)
                }
                ["moneyball", "crm", sub, ..] => {
                    assert!(has(&main, "crm") && has(&crm, sub), "{}", one)
                }
                ["moneyball", "ledger", ..] => assert!(has(&main, "ledger"), "{}", one),
                ["moneyball", "schedule", "--at", _] => {
                    assert!(has(&main, "schedule") && main.contains("at: Option<String>"))
                }
                other => panic!("no check for {:?}", other),
            }
        }
    }
}
