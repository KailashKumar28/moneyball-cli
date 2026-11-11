//! The brief: core's per-product rows and feasibility math as they
//! serialize (CRM figures null without CRM data), with the sentences
//! around the table worded here so the interface composes none.

use moneyball_core::brief::{self, Loaded, ProductRowsAndFeasibility};
use moneyball_core::text::commas;
use serde::Serialize;
use tauri::State;

use crate::error::AppError;
use crate::front_end::crm_note;
use crate::state::AppState;

#[derive(Serialize)]
pub struct BriefView {
    /// None when there is no snapshot, or it would not load (`fault`).
    pub brief: Option<ProductRowsAndFeasibility>,
    pub crm_note: Option<String>,
    /// A snapshot exists but would not load.
    pub fault: Option<AppError>,
    pub staleness: Option<String>,
    pub cpl_order: Option<String>,
    /// The one sentence under the table.
    pub feasibility_line: Option<String>,
}

fn feasibility_line(b: &ProductRowsAndFeasibility) -> String {
    let f = &b.feasibility;
    if !b.has_crm {
        return "Feasibility needs qualified leads, and this snapshot has no CRM data.".into();
    }
    format!(
        "{:.1} qualified a day at Rs.{} a day is Rs.{} per qualified. The goal is {:.0} a day.",
        f.tot_q_per_day,
        commas(f.tot_spend_per_day),
        commas(f.cur_rpq),
        f.tot_goal_per_day
    )
}

/// `crm_note`: the CRM sentence in this app's words
/// (`front_end::crm_note`), not the terminal's that `loaded` carries.
pub fn brief_view(loaded: Loaded, crm_note: Option<String>) -> BriefView {
    let b = loaded.brief.as_ref();
    BriefView {
        // Core marks the warning for a terminal; the interface has its own.
        staleness: b
            .and_then(|b| brief::staleness_warning(&b.snapshot_date))
            .map(|w| w.trim_start_matches("!! ").to_string()),
        cpl_order: b
            .and_then(|b| brief::cpl_order(&b.rows))
            .map(|order| format!("cheapest Meta lead first: {}", order)),
        feasibility_line: b.map(feasibility_line),
        crm_note,
        fault: loaded.fault.map(AppError::from),
        brief: loaded.brief,
    }
}

#[tauri::command(async)]
pub fn brief(state: State<'_, AppState>) -> Result<BriefView, AppError> {
    let cfg = state.workspace()?;
    let loaded = brief::load(&cfg);
    // Read again only when there is a sentence to reword.
    let note = loaded.crm_note.as_ref().and_then(|_| crm_note(&cfg));
    Ok(brief_view(loaded, note))
}
