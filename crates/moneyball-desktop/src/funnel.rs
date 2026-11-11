//! One product's funnel: core's rows as they serialize (CRM figures null
//! without CRM data), each with its verdict, and the notes under the
//! table worded here.

use moneyball_core::funnel::{self, By, FunnelRow, KILL_TABLE, LAG_HOURS};
use moneyball_core::{crm, product, snapshot, AppConfig};
use serde::Serialize;
use tauri::State;

use crate::error::AppError;
use crate::refresh::SNAPSHOT_DAYS;
use crate::state::AppState;

#[derive(Serialize)]
pub struct VerdictRow {
    #[serde(flatten)]
    pub row: FunnelRow,
    pub verdict: &'static str,
}

#[derive(Serialize)]
pub struct FunnelView {
    /// The product's name as configured.
    pub product: String,
    pub has_crm: bool,
    pub rows: Vec<VerdictRow>,
    pub cpl_ranking: Option<String>,
    /// The kill rule and what immature means, or why kill is withheld.
    pub notes: Vec<String>,
}

fn notes(has_crm: bool) -> Vec<String> {
    if !has_crm {
        return vec![
            "Kill is withheld: this snapshot has no CRM data, so qualified counts are unknown, not zero."
                .into(),
        ];
    }
    vec![
        format!(
            "Kill means spend passed {}x, {}x or {}x the target cost per qualified with 0, 1 or 2 qualified.",
            KILL_TABLE[0], KILL_TABLE[1], KILL_TABLE[2]
        ),
        format!(
            "Immature means the leads arrived in the last {} hours.",
            LAG_HOURS
        ),
    ]
}

pub fn funnel_view(
    cfg: &AppConfig,
    product: &str,
    days: u32,
    by: &str,
) -> Result<FunnelView, AppError> {
    let by = By::parse(by)?;
    let snap = snapshot::load(&cfg.snap_for(cfg.date.as_deref())?)?;
    let product =
        product::resolve_configured(cfg, product).map_err(|e| AppError::new("config", e))?;
    let rows = funnel::compute(&snap, cfg, &product, days.clamp(1, SNAPSHOT_DAYS), by);
    let has_crm = !crm::is_empty(&snap.crm);
    Ok(FunnelView {
        cpl_ranking: funnel::cpl_ranking(&rows),
        notes: notes(has_crm),
        rows: rows
            .into_iter()
            .map(|row| VerdictRow {
                verdict: funnel::verdict(&row),
                row,
            })
            .collect(),
        product,
        has_crm,
    })
}

#[tauri::command(async)]
pub fn funnel(
    state: State<'_, AppState>,
    product: String,
    days: u32,
    by: String,
) -> Result<FunnelView, AppError> {
    funnel_view(&state.workspace()?, &product, days, &by)
}
