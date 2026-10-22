//! The advisor's tools over the on-disk snapshot - brief, funnel,
//! diagnose, each returning what its typed command shows the model -
//! and recommend, which records into the ledger.
//! Split from commands.rs (size cap).

/// Tool executor over on-disk snapshot data. Runs on the agent worker
/// thread; errors become ToolOutput{is_error} messages the model sees.
pub(crate) struct SnapshotTools {
    pub(crate) cfg: moneyball_core::AppConfig,
}

impl SnapshotTools {
    fn snap(&self) -> Result<moneyball_core::Snapshot, String> {
        self.cfg
            .snap_for(self.cfg.date.as_deref())
            .and_then(|p| moneyball_core::snapshot::load(&p))
            .map_err(|_| {
                "no snapshot on disk - tell the user to run /refresh (or /brief, which \
                 self-heals) before asking for numbers"
                    .to_string()
            })
    }
}

impl moneyball_core::agent::ToolExec for SnapshotTools {
    fn run(&self, name: &str, args: &serde_json::Value) -> Result<String, String> {
        match name {
            "brief" => Ok(crate::brief_flow::brief_output(&self.cfg, &self.snap()?)),
            "funnel" => {
                let arg = |k: &str| args.get(k).and_then(|v| v.as_str());
                let by = moneyball_core::funnel::By::parse(arg("by").unwrap_or("adset"))
                    .map_err(|e| e.reason())?;
                let product = moneyball_core::product::resolve_configured(
                    &self.cfg,
                    arg("product").unwrap_or(""),
                )?;
                Ok(crate::funnel_flow::funnel_output(
                    &self.cfg,
                    &self.snap()?,
                    &product,
                    by,
                ))
            }
            "diagnose" => {
                let name = args.get("product").and_then(|v| v.as_str()).unwrap_or("");
                let product = moneyball_core::product::resolve_configured(&self.cfg, name)?;
                Ok(crate::diagnose_flow::diagnose_output(
                    &self.cfg,
                    &self.snap()?,
                    &product,
                ))
            }
            "recommend" => {
                let rec: moneyball_core::schema::Rec = serde_json::from_value(args.clone())
                    .map_err(|e| {
                        format!("not recorded: {} - fix the arguments and call again", e)
                    })?;
                let check = rec.prediction.verify_by.clone();
                match moneyball_core::ledger::submit(
                    &self.cfg,
                    &self.snap()?,
                    rec,
                    chrono::Local::now(),
                ) {
                    Ok(id) => Ok(format!(
                        "recorded {id}. Give the user its id. moneyball marks it applied when the \
                         change shows in the ad account's change log (or when they type \
                         /ledger {id} applied) and checks the prediction after {check}."
                    )),
                    Err(why) => Err(format!(
                        "not recorded - fix these and call recommend again, or drop it:\n- {}",
                        why.join("\n- ")
                    )),
                }
            }
            other => Err(format!(
                "tool \"{}\" is not implemented - answer from the tools you have",
                other
            )),
        }
    }
}
