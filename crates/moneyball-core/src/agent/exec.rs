//! The advisor's tools over the on-disk snapshot - brief, funnel,
//! diagnose, each returning what its typed command shows the model
//! (tool_text.rs) - and recommend, which records into the ledger.

use serde_json::Value;

use super::prompt::FrontEnd;
use super::tool_text;
use crate::config::AppConfig;
use crate::tools::{self, Tool};

/// The tools every advisor turn puts on the wire. Only ones `run`
/// below implements - the model must never be steered into stubs.
pub(super) fn tools() -> Vec<Tool> {
    vec![
        tools::brief_tool(),
        tools::funnel_tool(),
        tools::diagnose_tool(),
        tools::recommend_tool(),
    ]
}

/// Tool executor over on-disk snapshot data. Runs on the agent worker
/// thread; errors become ToolOutput{is_error} messages the model sees.
pub(super) struct SnapshotTools {
    cfg: AppConfig,
    /// What the model reads where a tool's answer names a next step, in
    /// the front-end's words (`FrontEnd::no_snapshot`, `::recorded`).
    no_snapshot: String,
    recorded: String,
}

impl SnapshotTools {
    pub(super) fn new(cfg: AppConfig, front: &FrontEnd) -> Self {
        Self {
            cfg,
            no_snapshot: front.no_snapshot(),
            recorded: front.recorded(),
        }
    }

    fn snap(&self) -> Result<crate::Snapshot, String> {
        self.cfg
            .snap_for(self.cfg.date.as_deref())
            .and_then(|p| crate::snapshot::load(&p))
            .map_err(|_| self.no_snapshot.clone())
    }

    fn product(&self, args: &Value) -> Result<String, String> {
        let name = args.get("product").and_then(|v| v.as_str()).unwrap_or("");
        crate::product::resolve_configured(&self.cfg, name)
    }

    fn recommend(&self, args: &Value) -> Result<String, String> {
        let rec: crate::schema::Rec = serde_json::from_value(args.clone())
            .map_err(|e| format!("not recorded: {} - fix the arguments and call again", e))?;
        let check = rec.prediction.verify_by.clone();
        match crate::ledger::submit(&self.cfg, &self.snap()?, rec, chrono::Local::now()) {
            Ok(id) => Ok(self
                .recorded
                .replace("{id}", &id)
                .replace("{check}", &check)),
            Err(why) => Err(format!(
                "not recorded - fix these and call recommend again, or drop it:\n- {}",
                why.join("\n- ")
            )),
        }
    }
}

impl super::ToolExec for SnapshotTools {
    fn run(&self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "brief" => Ok(tool_text::brief_output(&self.cfg, &self.snap()?)),
            "funnel" => {
                let by = args.get("by").and_then(|v| v.as_str()).unwrap_or("adset");
                let by = crate::funnel::By::parse(by).map_err(|e| e.reason())?;
                let product = self.product(args)?;
                Ok(tool_text::funnel_output(
                    &self.cfg,
                    &self.snap()?,
                    &product,
                    by,
                ))
            }
            "diagnose" => {
                let product = self.product(args)?;
                Ok(tool_text::diagnose_output(
                    &self.cfg,
                    &self.snap()?,
                    &product,
                ))
            }
            "recommend" => self.recommend(args),
            other => Err(format!(
                "tool \"{}\" is not implemented - answer from the tools you have",
                other
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::prompt::{self, app_state_block};
    use super::super::ToolExec;
    use super::*;

    const TUI: FrontEnd = FrontEnd {
        commands: &["/brief", "/refresh"],
        output_style: "Output style: plain.",
        refresh: "/refresh",
        setup: "/setup",
        brief: "/brief",
        mark_applied: "type /ledger {id} applied",
    };

    /// A front-end with buttons and no typed commands.
    const DESKTOP: FrontEnd = FrontEnd {
        commands: &[],
        output_style: "Output style: markdown renders here.",
        refresh: "the Refresh button",
        setup: "`moneyball` in a terminal",
        brief: "the Brief screen",
        mark_applied: "run `moneyball ledger {id} applied` in a terminal",
    };

    fn cfg() -> AppConfig {
        AppConfig::resolve_optional(Some("/nonexistent/mb-exec-test"), None)
    }

    fn exec() -> SnapshotTools {
        SnapshotTools::new(cfg(), &TUI)
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

    /// The hard rule (CLAUDE.md): the model never reads a command the
    /// front-end does not have. Everything a desktop-style front-end's
    /// model is told - the system prompt and the tools' own sentences -
    /// carries that front-end's words and no slash command.
    #[test]
    fn a_front_end_without_commands_gets_no_slash_command() {
        let mut with_workspace = cfg();
        with_workspace.workspace = Some(serde_json::from_str(r#"{"products":[]}"#).unwrap());
        let told = [
            prompt::AGENT_SYSTEM_PROMPT.to_string(),
            prompt::RECORD_RULE.to_string(),
            app_state_block(&cfg(), None, None, &DESKTOP),
            app_state_block(&with_workspace, None, None, &DESKTOP),
            app_state_block(&with_workspace, Some("2026-11-11"), None, &DESKTOP),
            DESKTOP.no_snapshot(),
            DESKTOP.recorded(),
        ];
        for text in &told {
            assert_eq!(slash_commands(text), Vec::<&str>::new(), "in: {}", text);
        }
        assert!(told[2].contains("NOT configured - `moneyball` in a terminal is the fix"));
        assert!(told[5].contains("run the Refresh button (or the Brief screen,"));
        // The check itself sees the TUI's commands.
        assert_eq!(slash_commands(&TUI.no_snapshot()), ["/refresh", "/brief"]);
        assert_eq!(slash_commands(&TUI.recorded()), ["/ledger"]);
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

    #[test]
    fn failures_are_messages_for_the_model() {
        let exec = exec();
        let no_snap = exec.run("brief", &serde_json::json!({})).unwrap_err();
        assert_eq!(
            no_snap,
            "no snapshot on disk - tell the user to run /refresh (or /brief, which \
             self-heals) before asking for numbers"
        );
        let unknown = exec.run("teleport", &serde_json::json!({})).unwrap_err();
        assert!(unknown.contains("\"teleport\" is not implemented"));
    }

    /// The TUI's recommend answer, word for word as before the phrases
    /// came from the front-end.
    #[test]
    fn a_recorded_recommendation_names_its_id_and_check_date() {
        let mut exec = exec();
        exec.recorded = TUI.recorded();
        let said = exec
            .recorded
            .replace("{id}", "r7")
            .replace("{check}", "2026-11-20");
        assert_eq!(
            said,
            "recorded r7. Give the user its id. moneyball marks it applied when the \
             change shows in the ad account's change log (or when they type \
             /ledger r7 applied) and checks the prediction after 2026-11-20."
        );
    }
}
