//! Executor tests, and the rule that a front-end without typed commands
//! never has one named to its model. Split from exec.rs (size cap).

use super::super::prompt::{self, app_state_block, FrontEnd};
use super::super::{tool_text, ToolExec};
use super::*;
use crate::crm::status::CrmHealth;

const TUI: FrontEnd = FrontEnd {
    commands: &["/brief", "/refresh"],
    output_style: "Output style: plain.",
    say: |step| step.to_string(),
    self_heal: Some("/brief"),
};

/// A front-end with buttons and no typed commands.
const DESKTOP: FrontEnd = FrontEnd {
    commands: &[],
    output_style: "Output style: markdown renders here.",
    say: |step| match step {
        "/refresh" => "the Refresh button".to_string(),
        other => match other.strip_prefix('/') {
            Some(cmd) => format!("moneyball {} in Terminal", cmd),
            None => format!("{} in Terminal", other),
        },
    },
    self_heal: None,
};

fn cfg() -> AppConfig {
    AppConfig::resolve_optional(Some("/nonexistent/mb-exec-test"), None)
}

fn exec() -> SnapshotTools {
    SnapshotTools {
        cfg: cfg(),
        say: TUI.say,
        self_heal: TUI.self_heal,
    }
}

/// A snapshot with ads but no CRM data and no change log, in a
/// workspace with no crm.toml: every tool has a fix to name.
fn bare_snapshot() -> crate::Snapshot {
    crate::Snapshot {
        path: "/nonexistent/mb-exec-test/snap".into(),
        date: "2026-11-11".into(),
        ads_daily: vec![],
        adsets: serde_json::json!({}),
        creatives: None,
        crm: serde_json::json!({}),
        leads: None,
        crm_contacts: None,
        regions: serde_json::json!([]),
        changes: None,
        ads_7d: None,
        campaigns: serde_json::json!([]),
    }
}

/// Words that are a typed command: "/refresh", "(/brief,", "`/setup`".
fn slash_commands(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/'))
        .filter(|w| {
            w.strip_prefix('/')
                .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
        })
        .collect()
}

fn crm_states() -> Vec<CrmHealth> {
    let failed = |fix: Option<&str>| CrmHealth::NoData {
        last: None,
        failed: Some(("401".into(), fix.map(String::from))),
    };
    let no_data = |last: Option<&str>| CrmHealth::NoData {
        last: last.map(String::from),
        failed: None,
    };
    vec![
        CrmHealth::NoSpec,
        failed(Some("/connect crm")),
        failed(Some("/setup meta")),
        failed(Some("moneyball crm secret api_key")),
        failed(None),
        no_data(Some("2026-11-01")),
        no_data(None),
    ]
}

/// The hard rule (CLAUDE.md): the model never reads a command the
/// front-end does not have. Everything a desktop-style front-end's
/// model is told - the system prompt, the tools' own sentences, the
/// CRM note in every state, the tool outputs - carries that
/// front-end's words and no slash command.
#[test]
fn a_front_end_without_commands_gets_no_slash_command() {
    let say = DESKTOP.say;
    let mut with_workspace = cfg();
    with_workspace.workspace = Some(serde_json::from_str(r#"{"products":[]}"#).unwrap());
    let snap = bare_snapshot();
    let by = crate::funnel::By::Adset;
    let mut told = vec![
        prompt::AGENT_SYSTEM_PROMPT.to_string(),
        prompt::RECORD_RULE.to_string(),
        app_state_block(&cfg(), None, None, &DESKTOP),
        app_state_block(&with_workspace, None, None, &DESKTOP),
        tool_text::no_snapshot(&say, DESKTOP.self_heal),
        recorded(say, "r7", "2026-11-20"),
        tool_text::brief_output(&with_workspace, &snap, &say),
        tool_text::funnel_output(&with_workspace, &snap, "Namma Mane", by, &say),
        tool_text::diagnose_output(&with_workspace, &snap, "Namma Mane", &say),
    ];
    for health in crm_states() {
        let note = health.note_for(&say).unwrap();
        told.push(app_state_block(
            &with_workspace,
            Some("2026-11-11"),
            Some(&note),
            &DESKTOP,
        ));
        told.push(note);
    }
    for text in &told {
        assert_eq!(slash_commands(text), Vec::<&str>::new(), "in: {}", text);
        // Nothing here self-heals, so nothing may be said to.
        assert!(!text.contains("self-heal"), "in: {}", text);
    }
    assert!(told[2].contains("NOT configured - moneyball setup in Terminal is the fix"));
    assert_eq!(
        told[4],
        "no snapshot on disk - tell the user to run the Refresh button before asking for numbers"
    );
    assert!(told[5].contains("when they type moneyball ledger r7 applied in Terminal)"));
    // The tool outputs do carry the notes being checked.
    assert!(told[6].contains("IMPORTANT: no CRM connected"));
    assert!(told[7].contains("Fix: moneyball connect crm in Terminal"));
    assert!(told[8].contains("the next the Refresh button pulls it"));
    // And the check itself sees the TUI's commands.
    assert_eq!(
        slash_commands(&tool_text::no_snapshot(&TUI.say, TUI.self_heal)),
        ["/refresh", "/brief"]
    );
    let tui_diagnose = tool_text::diagnose_output(&with_workspace, &snap, "x", &TUI.say);
    assert_eq!(slash_commands(&tui_diagnose), ["/refresh", "/connect"]);
}

/// Every tool on the wire has a handler: none falls through to
/// "not implemented" (the stub rule, CLAUDE.md).
#[test]
fn every_advertised_tool_is_handled() {
    let exec = exec();
    for t in tools() {
        let out = exec.run(&t.name, &serde_json::json!({}));
        let text = out.unwrap_or_else(|e| e);
        assert!(!text.contains("is not implemented"), "{}: {}", t.name, text);
    }
}

/// The TUI's tool sentences, word for word as before the front-end
/// worded them.
#[test]
fn the_tuis_tool_sentences_are_unchanged() {
    let exec = exec();
    let no_snap = exec.run("brief", &serde_json::json!({})).unwrap_err();
    assert_eq!(
        no_snap,
        "no snapshot on disk - tell the user to run /refresh (or /brief, which \
         self-heals) before asking for numbers"
    );
    assert_eq!(
        recorded(TUI.say, "r7", "2026-11-20"),
        "recorded r7. Give the user its id. moneyball marks it applied when the \
         change shows in the ad account's change log (or when they type \
         /ledger r7 applied) and checks the prediction after 2026-11-20."
    );
    let unknown = exec.run("teleport", &serde_json::json!({})).unwrap_err();
    assert!(unknown.contains("\"teleport\" is not implemented"));
}
