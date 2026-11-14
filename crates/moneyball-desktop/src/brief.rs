//! The brief: core's per-product rows and feasibility math as they
//! serialize (CRM figures null without CRM data), with the sentences
//! around the table worded here so the interface composes none.

use chrono::NaiveDate;
use moneyball_core::brief::{self, Loaded, ProductRowsAndFeasibility};
use moneyball_core::text::commas;
use serde::Serialize;
use tauri::State;

use crate::error::AppError;
use crate::front_end::crm_note;
use crate::state::AppState;
use crate::today::capitalized;

#[derive(Serialize)]
pub struct BriefView {
    /// None when there is no snapshot, or it would not load (`fault`).
    pub brief: Option<ProductRowsAndFeasibility>,
    /// Said in front of `crm_note` when the table has no CRM columns.
    pub crm_head: Option<String>,
    pub crm_note: Option<String>,
    /// A snapshot exists but would not load.
    pub fault: Option<AppError>,
    pub staleness: Option<String>,
    pub cpl_order: Option<String>,
    /// What the table's columns count; None with no brief.
    pub caption: Option<String>,
    /// The one sentence under the table.
    pub feasibility_line: Option<String>,
}

fn caption(b: &ProductRowsAndFeasibility) -> String {
    if b.has_crm {
        "Meta, CRM and Qual. are leads over the 7 days. Gap is qualified per day short of the goal."
    } else {
        "Meta is leads over the 7 days. Goal is qualified per day."
    }
    .into()
}

fn feasibility_line(b: &ProductRowsAndFeasibility) -> String {
    let f = &b.feasibility;
    if !b.has_crm {
        return "Feasibility needs qualified leads, and this snapshot has no CRM data.".into();
    }
    let Some(goal) = f.tot_goal_per_day else {
        return format!("Feasibility {}.", brief::print::NEEDS_A_GOAL);
    };
    let left_out = brief::no_goal_note(&b.rows)
        .map(|n| format!(" {}.", capitalized(&n)))
        .unwrap_or_default();
    format!(
        "{:.1} qualified a day at Rs.{} a day is Rs.{} per qualified. The goal is {:.0} a day.{}",
        f.tot_q_per_day,
        commas(f.tot_spend_per_day),
        commas(f.cur_rpq),
        goal,
        left_out
    )
}

fn staleness(snapshot_date: &str) -> Option<String> {
    let days = brief::stale_days(snapshot_date)?;
    let pulled = NaiveDate::parse_from_str(snapshot_date, "%Y-%m-%d").ok()?;
    Some(format!(
        "These numbers were pulled {} days ago, on {}. Refresh before acting on them.",
        days,
        pulled.format("%d %b")
    ))
}

/// `crm_note`: the CRM sentence in this app's words
/// (`front_end::crm_note`), not the terminal's that `loaded` carries.
pub fn brief_view(loaded: Loaded, crm_note: Option<String>) -> BriefView {
    let b = loaded.brief.as_ref();
    BriefView {
        // Core words the warning for a terminal; this one names the button.
        staleness: b.and_then(|b| staleness(&b.snapshot_date)),
        cpl_order: b.and_then(|b| brief::cpl_order(&b.rows)),
        caption: b.map(caption),
        feasibility_line: b.map(feasibility_line),
        crm_head: b
            .filter(|b| !b.has_crm)
            .map(|_| "CRM columns hidden.".into()),
        // Core's sentence starts lower case: here it follows the head.
        crm_note: crm_note.map(|n| capitalized(&n)),
        // No snapshot at all is said, with the button that pulls one.
        fault: match (&loaded.brief, loaded.fault) {
            (_, Some(e)) => Some(AppError::from(e)),
            (None, None) => Some(AppError::no_data()),
            (Some(_), None) => None,
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::demo;

    /// A product with no goal crosses as null, and the sentence under the
    /// table says it is left out of the totals (D7).
    #[test]
    fn a_product_without_a_goal_is_left_out_and_said() {
        let mut cfg = demo::workspace("brief-goal", true, true);
        let goals = &mut cfg.workspace.as_mut().unwrap().goals;
        goals.remove("Cityville");
        let v = brief_view(brief::load(&cfg), None);
        cfg.workspace.as_mut().unwrap().goals.clear();
        let none = brief_view(brief::load(&cfg), None);
        demo::remove(&cfg);
        let json = serde_json::to_value(&v).unwrap();
        assert!(json["brief"]["rows"][1]["goal"].is_null(), "{}", json);
        assert_eq!(json["brief"]["feasibility"]["tot_goal_per_day"], 6.0);
        let line = v.feasibility_line.unwrap();
        let said = " The goal is 6 a day. No goal set for 1 product - left out of these totals.";
        assert!(line.ends_with(said), "{}", line);
        let line = none.feasibility_line.unwrap();
        assert_eq!(
            line,
            "Feasibility needs a goal, and no product has one set."
        );
    }
}
