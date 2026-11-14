//! The one failure shape every command returns: a kind the interface can
//! branch on, a sentence to show, and the action that fixes it when one
//! exists. Never a Debug form, a source chain or a secret.

use moneyball_core::words::Command;
use serde::Serialize;

use crate::action::{translate, Action};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{reason}")]
pub struct AppError {
    pub kind: String,
    pub reason: String,
    pub fix: Option<Action>,
}

impl AppError {
    pub fn new(kind: &str, reason: impl Into<String>) -> Self {
        AppError {
            kind: kind.into(),
            reason: reason.into(),
            fix: None,
        }
    }

    /// Refused because something else is running.
    pub fn busy(reason: &str) -> Self {
        AppError::new("busy", reason)
    }

    pub fn no_workspace() -> Self {
        AppError::new("no_workspace", "No workspace is open.")
    }

    /// Nothing has been pulled for this workspace yet: what every screen
    /// that reads the data says then, with the button that pulls it.
    pub fn no_data() -> Self {
        AppError::new("snapshot", "No data yet: nothing has been pulled for this client.").with_fix(
            "/refresh",
            "pulls Meta, then CRM, then writes yesterday's report (about 2 minutes)",
        )
    }

    pub fn with_fix(mut self, command: &str, does: &str) -> Self {
        self.fix = Some(translate(&Command::new(command, does)));
        self
    }
}

impl From<moneyball_core::Error> for AppError {
    fn from(e: moneyball_core::Error) -> Self {
        // Core's sentence for this one tells a developer to copy an
        // example file; the app has no setup, so it names the terminal.
        if let moneyball_core::Error::NoWorkspaceConfig { .. } = e {
            return AppError::new(
                "no_workspace_config",
                "This folder is not a Moneyball workspace: it has no .moneyball/config.json.",
            )
            .with_fix("/setup", "it sets the workspace up; run it in that folder");
        }
        // Core's sentence names a date pattern and a folder path.
        if let moneyball_core::Error::NoSnapshot { .. } = e {
            return AppError::no_data();
        }
        AppError {
            kind: e.kind().into(),
            reason: e.reason(),
            fix: e.fix().map(|f| translate(&Command::fix(&f))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_core_error_keeps_its_kind_and_gets_its_fix_translated() {
        let auth: AppError = moneyball_core::Error::CrmAuth {
            message: "the CRM said 401".into(),
            fix: "moneyball crm secret lz".into(),
        }
        .into();
        assert_eq!(auth.kind, "crm_auth");
        assert_eq!(auth.reason, "the CRM said 401");
        assert!(
            matches!(&auth.fix, Some(Action::Terminal { command, .. }) if command == "moneyball crm secret lz")
        );
        let none: AppError = moneyball_core::Error::NoWorkspaceConfig { path: "/x".into() }.into();
        assert_eq!(none.kind, "no_workspace_config");
        assert!(!none.reason.contains("example"), "{}", none.reason);
        let json = serde_json::to_value(AppError::busy("wait")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "busy", "reason": "wait", "fix": null})
        );
    }
}
