//! `runs/<date>.json` - what one refresh did: every step, its counts, and
//! for a failure the kind and the one command that fixes it. The daily
//! refresh writes it; the landing, `/status` and the scheduler read it.
//! See mod.rs for the versioning policy.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One refresh run (`moneyball.run/<major>`), written even when it fails.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunRecord {
    pub schema: String,
    /// The workspace's stable id (config.json), the tenant key.
    pub workspace_id: String,
    /// The snapshot date the run filled (YYYY-MM-DD); the data covers the
    /// day before.
    pub date: String,
    pub trigger: Trigger,
    /// RFC3339 UTC.
    pub started_at: String,
    /// RFC3339 UTC; empty while running.
    #[serde(default)]
    pub finished_at: String,
    pub outcome: Outcome,
    pub steps: Vec<RunStep>,
    /// Files the run wrote, by role ("snapshot", "report", ...).
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Schedule,
    Manual,
}

/// `failed`: nothing new was written (Meta failed). `degraded`: written,
/// but something is missing (CRM failed, or a product). `ok`: complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    Ok,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunStep {
    /// `meta`, `assets`, `crm`, `report`.
    pub name: String,
    pub status: StepStatus,
    pub ms: u64,
    /// What the step moved: rows, leads, tickets, pages, ...
    #[serde(default)]
    pub counts: BTreeMap<String, u64>,
    /// Machine-readable failure kind (`meta_auth`, `crm_auth`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    /// One plain sentence, scrubbed of secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The one command that fixes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// Carried over from an earlier run today rather than done again (a
    /// Meta pull reused without --force): its status and counts are that
    /// run's, so a missing product stays missing on screen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Done,
    /// Done for some products, not all.
    Partial,
    Failed,
    Skipped,
}
