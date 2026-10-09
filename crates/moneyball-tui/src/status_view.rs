//! /status (frame R6): the last 14 data days as a coloured table - ok
//! green, none red, gaps yellow, missed days dim - then how Meta and the
//! CRM last answered and where the run records live. The rows and their
//! words come from moneyball_core::status, the same as `moneyball status`.

use ratatui::style::Color;

use moneyball_core::status::{self, Mark, Row, Tone, HEADER, WIDTHS};

use crate::chat::{cells, Cell};
use crate::widgets::{bold, dim, fg, gray, Para};
use crate::App;

const INDENT: usize = 6;

pub(crate) fn run_status(app: &mut App) {
    let started = std::time::Instant::now();
    let days = status::days(&app.cfg, 14);
    let ran_at = INDENT + WIDTHS.iter().sum::<usize>();
    let mut paras = vec![
        Para::new(2, 2)
            .run("\u{23BF} ", dim())
            .run("\u{25CF}", fg(Color::Green))
            .run(" moneyball ", dim())
            .run("status", bold(Color::Cyan)),
        Para::default(),
    ];
    let mut header = Para::new(INDENT, ran_at);
    for (w, h) in WIDTHS.iter().zip(HEADER) {
        header = header.run(format!("{:<w$}", h), dim());
    }
    paras.push(header.run(HEADER[5], dim()));
    for row in status::rows(&days) {
        paras.push(match row {
            Row::Day(d) => {
                let missed = d.tone == Tone::Missed;
                let mut p = Para::new(INDENT, ran_at).run(
                    format!(
                        "{:<w$}",
                        d.date.format("%a %d %b").to_string(),
                        w = WIDTHS[0]
                    ),
                    gray(),
                );
                for (w, m) in WIDTHS[1..]
                    .iter()
                    .zip([&d.meta, &d.crm, &d.leads, &d.report])
                {
                    let style = match m {
                        _ if missed => dim(),
                        Mark::Ok => fg(Color::Green),
                        Mark::None => fg(Color::Red),
                        Mark::Warn(_) => fg(Color::Yellow),
                        Mark::Absent => dim(),
                    };
                    p = p.run(format!("{:<w$}", m.word()), style);
                }
                let ran_style = match d.tone {
                    Tone::Normal => gray(),
                    Tone::Missed => dim(),
                    Tone::Failed => fg(Color::Red),
                };
                p.run(d.ran.clone(), ran_style)
            }
            Row::Missed { count, from, to } => {
                Para::new(INDENT, INDENT).run(status::missed_line(count, from, to), dim())
            }
        });
    }
    paras.push(Para::default());
    for (key, value) in status::footer(&app.cfg, &days) {
        paras.push(
            Para::new(INDENT, INDENT + 11)
                .run(format!("{:<11}", key), gray())
                .run(value, gray()),
        );
    }
    let ms = started.elapsed().as_millis() as u64;
    let took = if ms < 1000 {
        format!("({}ms)", ms)
    } else {
        format!("({})", moneyball_core::refresh::duration(ms))
    };
    paras.push(Para::new(INDENT, INDENT).run(took, fg(Color::Green)));
    app.chat.push(Cell::Rich(cells::Rich(paras)));
}
