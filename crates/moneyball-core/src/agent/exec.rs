//! The advisor's tools over the on-disk snapshot - brief, funnel,
//! diagnose, each returning what its typed command shows the model
//! (tool_text.rs) - and recommend, which records into the ledger.

use serde_json::Value;

use super::prompt::Say;
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
    pub(super) cfg: AppConfig,
    /// Words every step a tool's answer names (`FrontEnd::say`).
    pub(super) say: Say,
}

/// The tools' answer when there is no snapshot to read.
fn no_snapshot(say: Say) -> String {
    format!(
        "no snapshot on disk - tell the user to run {} (or {}, which \
         self-heals) before asking for numbers",
        say("/refresh"),
        say("/brief")
    )
}

/// The recommend tool's answer for a recorded recommendation.
fn recorded(say: Say, id: &str, check: &str) -> String {
    format!(
        "recorded {id}. Give the user its id. moneyball marks it applied when the \
         change shows in the ad account's change log (or when they type \
         {}) and checks the prediction after {check}.",
        say(&format!("/ledger {id} applied"))
    )
}

impl SnapshotTools {
    fn snap(&self) -> Result<crate::Snapshot, String> {
        self.cfg
            .snap_for(self.cfg.date.as_deref())
            .and_then(|p| crate::snapshot::load(&p))
            .map_err(|_| no_snapshot(self.say))
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
            Ok(id) => Ok(recorded(self.say, &id, &check)),
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
            "brief" => Ok(tool_text::brief_output(&self.cfg, &self.snap()?, &self.say)),
            "funnel" => {
                let by = args.get("by").and_then(|v| v.as_str()).unwrap_or("adset");
                let by = crate::funnel::By::parse(by).map_err(|e| e.reason())?;
                let product = self.product(args)?;
                Ok(tool_text::funnel_output(
                    &self.cfg,
                    &self.snap()?,
                    &product,
                    by,
                    &self.say,
                ))
            }
            "diagnose" => {
                let product = self.product(args)?;
                Ok(tool_text::diagnose_output(
                    &self.cfg,
                    &self.snap()?,
                    &product,
                    &self.say,
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
#[path = "exec_tests.rs"]
mod tests;
