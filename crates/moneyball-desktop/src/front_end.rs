//! What the advisor is told about this front-end: it has no commands,
//! markdown renders, and each step it may suggest is worded for an app
//! with buttons. The one place the desktop's `FrontEnd` is written.

use moneyball_core::agent::prompt::{self, FrontEnd};
use moneyball_core::crm::status::CrmHealth;
use moneyball_core::{snapshot, AppConfig};

pub const DESKTOP: FrontEnd<'static> = FrontEnd {
    commands: &[],
    output_style: "Output style: markdown renders here - tables, lists and bold are fine. \
                   No HTML.",
    say: crate::action::say,
    // The Brief screen only reads: with no snapshot it has nothing to
    // show, and the model must not send anyone there to wait.
    self_heal: None,
};

/// The CRM sentence for the snapshot on screen with every fix said this
/// app's way (core's `Loaded.crm_note` words them for the terminal).
/// None when the CRM data is fresh or there is no snapshot.
// ponytail: this reads the snapshot a second time (brief::load already
// did). If it shows, core's `Loaded` should carry the `CrmHealth`.
pub fn crm_note(cfg: &AppConfig) -> Option<String> {
    let snap = snapshot::load(&cfg.snap_for(cfg.date.as_deref()).ok()?).ok()?;
    CrmHealth::of(cfg, &snap).note_for(&DESKTOP.say)
}

/// The system prompt of a free-form advisor turn: the persona, then the
/// live app state for the snapshot on screen.
pub fn system_prompt(cfg: &AppConfig, snap_date: Option<&str>, crm_note: Option<&str>) -> String {
    format!(
        "{}\n\n{}",
        prompt::AGENT_SYSTEM_PROMPT,
        prompt::app_state_block(cfg, snap_date, crm_note, &DESKTOP)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::tests::slash_command;

    #[test]
    fn the_prompt_names_no_slash_command_and_says_markdown_renders() {
        let cfg = AppConfig::resolve_optional(Some("nonexistent-mb-desktop-prompt"), None);
        for snap in [None, Some("2026-10-09")] {
            let s = system_prompt(&cfg, snap, None);
            let state = s.split("== live app state").nth(1).unwrap();
            assert!(state.contains("slash commands: none"), "{}", state);
            assert!(state.contains("markdown renders here"), "{}", state);
            assert_eq!(slash_command(state), None, "{}", state);
            assert!(!state.contains("self-heal"), "{}", state);
        }
        let none = system_prompt(&cfg, None, None);
        assert!(
            none.contains("snapshot: none yet. Refresh (Cmd-R) pulls one when"),
            "{}",
            none
        );
    }
}
