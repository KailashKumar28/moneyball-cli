//! /ledger: the advisor's recommendations, what each predicted and how
//! it came out, caught up with the newest snapshot first;
//! /ledger <id> applied|ignored marks one by hand. The same text as
//! `moneyball ledger`.

use moneyball_core::ledger;

use crate::*;

pub(crate) fn run_ledger(app: &mut App, arg: &str) {
    let started = std::time::Instant::now();
    let today = chrono::Local::now().date_naive();
    let words: Vec<&str> = arg.split_whitespace().collect();
    let (ok, lines): (bool, Vec<String>) = match words.as_slice() {
        [] => {
            let mut out = Vec::new();
            if let Err(e) = ledger::catch_up(&app.cfg, today) {
                out.push(format!("could not update the ledger: {}", e));
            }
            let text = ledger::text(&ledger::load(&app.cfg));
            out.extend(text.lines().map(String::from));
            (true, out)
        }
        [id, state] => match ledger::by_hand(state) {
            Some(s) => match ledger::mark(&app.cfg, id, s, None, today) {
                Ok(()) => (true, vec![format!("{} marked {}", id, ledger::name(s))]),
                Err(e) => (false, vec![e]),
            },
            None => (false, vec!["usage: /ledger <id> applied|ignored".into()]),
        },
        _ => (
            false,
            vec!["usage: /ledger, or /ledger <id> applied|ignored".into()],
        ),
    };
    app.chat.push_tool(
        "ledger",
        arg,
        lines,
        ok,
        started.elapsed().as_millis() as u64,
    );
}
