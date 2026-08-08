//! /crm - CRM status as a tool cell. Thin wrapper over
//! core crm::status (pure file reads, instant, no worker thread).

use crate::*;

pub(crate) fn run_crm_status(app: &mut App) {
    let started = std::time::Instant::now();
    match moneyball_core::crm::status::status_lines(&app.cfg) {
        Ok(lines) => app.chat.push_tool(
            "crm status",
            "",
            lines,
            true,
            started.elapsed().as_millis() as u64,
        ),
        Err(e) => app.chat.push_tool(
            "crm status",
            "",
            vec![format!("{}", e)],
            false,
            started.elapsed().as_millis() as u64,
        ),
    }
}
