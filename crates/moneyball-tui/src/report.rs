//! `/debug [comment]` - file a bug report for the current session.
//!
//! The user saw wrong analysis or a hallucination; this registers the
//! session in ~/.moneyball/reports/ at the exact complaint point: a
//! marker item goes into the history/session file (model-facing on
//! resume, never shown as a user message), the session file is frozen
//! as evidence, and a report record stores the comment for the admin's
//! `moneyball debug --reports` review pass.

use crate::app::App;
use crate::chat::{cells, Cell};
use moneyball_core::agent::Item;
use moneyball_core::{debug, session};

pub(crate) fn run_debug_report(app: &mut App, comment: &str) {
    let Some(id) = app.session.as_ref().map(|log| log.meta.id.clone()) else {
        app.chat.push(Cell::System(cells::System(
            "no active session to report (session persistence is off).".into(),
        )));
        return;
    };
    // Marker goes through record() so the in-memory history and the
    // session file stay the same transcript (section 6b invariant).
    app.record(Item::User {
        text: debug::marker_text(comment),
    });
    let n = app.history.len();
    let outcome = session::read_raw(&id, app.cfg.sessions_root())
        .and_then(|(_, raw)| debug::register(&id, &raw, n, comment));
    app.chat.push(Cell::System(cells::System(match outcome {
        Ok(path) => format!(
            "bug report filed for this session ({}). the admin reviews it with: moneyball debug --reports",
            path.display()
        ),
        Err(e) => format!("could not file the bug report: {}", e),
    })));
}
