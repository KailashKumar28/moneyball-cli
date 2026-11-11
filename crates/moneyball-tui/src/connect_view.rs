//! /connect: where every source stands - ads, leads, advisor - with its
//! state coloured (connected green, refused red, missing yellow) and the
//! one command that fixes it. The rows are moneyball_core::connections,
//! the same as `moneyball connect`. Replaces /keychain (model keys show
//! here).

use ratatui::style::Color;

use moneyball_core::connections::{self, Link};

use crate::chat::{cells, Cell};
use crate::widgets::{bold, dim, fg, gray, Para, OK};
use crate::App;

pub(crate) fn run_connect(app: &mut App) {
    let mut paras = vec![
        Para::new(2, 2)
            .run("\u{23BF} ", dim())
            .run(OK, fg(Color::Green))
            .run(" moneyball ", dim())
            .run("connect", bold(Color::Cyan)),
        Para::default(),
    ];
    // group 10, name 12, state 13: the detail wraps under itself.
    const NAME_AT: usize = 6 + 10;
    const DETAIL_AT: usize = NAME_AT + 12 + 13;
    for s in connections::status_all(&app.cfg) {
        let (word, color) = match s.link {
            Link::Connected => ("connected", Color::Green),
            Link::Refused => ("refused", Color::Red),
            Link::Missing => ("missing", Color::Yellow),
        };
        paras.push(
            Para::new(6, DETAIL_AT)
                .run(format!("{:<10}", s.group.word()), dim())
                .run(format!("{:<12}", s.name), gray())
                .run(format!("{} {:<11}", OK, word), fg(color))
                .run(s.detail, gray()),
        );
        // The fix starts under the name, not the detail: a command
        // that wraps mid-way cannot be copied whole.
        if let Some(fix) = s.fix {
            paras.push(
                Para::new(NAME_AT, NAME_AT + "fix:  ".len())
                    .run("fix:  ", gray())
                    .run(fix.clone(), bold(Color::Cyan))
                    .run(
                        format!("   {}", moneyball_core::words::fix_does(&fix)),
                        gray(),
                    ),
            );
        }
    }
    app.chat.push(Cell::Rich(cells::Rich(paras)));
}
