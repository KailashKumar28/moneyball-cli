//! `.moneyball/ledger.jsonl` - the prediction ledger, one event per line,
//! append-only. A recommendation is proposed with the number it predicts;
//! later events say whether it was applied and how the prediction came
//! out. A rec's state is the last event that names it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One line of ledger.jsonl.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LedgerEvent {
    /// `moneyball.ledger/<major>`.
    pub schema: String,
    /// The recommendation this is about, e.g. `mb-20261022-1`.
    pub id: String,
    pub event: RecState,
    /// YYYY-MM-DD the event happened.
    pub date: String,
    /// "advisor", "you" or "moneyball" (its own sweep).
    pub by: String,
    /// On `proposed`: the moment (RFC 3339), so a change logged earlier
    /// that day is not taken for its being applied.
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Only on `proposed`: the recommendation itself.
    #[serde(default)]
    pub rec: Option<Rec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecState {
    Proposed,
    Applied,
    Ignored,
    /// Never applied by its verify-by date.
    Expired,
    Confirmed,
    Refuted,
    /// Its metric could not be measured (the entity stopped, or the CRM
    /// data it needs is missing).
    Inconclusive,
}

/// What the advisor recommended, and what it said would happen.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Rec {
    pub product: String,
    /// "campaign", "adset", "ad" or "product".
    pub level: String,
    /// The Meta id (the product's name at product level).
    pub entity_id: String,
    /// Filled from the data when recorded.
    #[serde(default)]
    pub entity_name: String,
    /// budget, creative, targeting, structure, form or crm.
    pub lever: String,
    /// The change, in words.
    pub action: String,
    /// The 7-day numbers it rests on, as checked when it was proposed.
    pub evidence: Vec<Evidence>,
    pub prediction: Prediction,
    #[serde(default)]
    pub budget: Option<BudgetMove>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    /// spend, meta_leads, cpl, crm_leads, qualified, cost_per_qualified,
    /// l_to_q.
    pub metric: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Prediction {
    pub metric: String,
    /// "up" or "down".
    pub direction: String,
    pub target: f64,
    /// YYYY-MM-DD: measured over the days from the change to this one.
    pub verify_by: String,
}

/// Rupees a day, before and after.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BudgetMove {
    pub from: f64,
    pub to: f64,
    /// A move over 20% restarts Meta's learning; true accepts that.
    #[serde(default)]
    pub relearn: bool,
}
