# HANDOFF.md - moneyball-cli

Running notes. Prune aggressively; this file ages in days, not months. Each
entry: 1-3 lines, dated, with the decision and the why.

## 2026-07-14 - paste handling

`crossterm` 0.28 emits `Event::Paste(String)` for clipboard pastes, NOT a stream
of key events. The event loop must match on `Event::Paste` and route the string
to the focused input field. Dropping Paste events silently breaks the
Meta-access-token paste in setup. See `handle_paste` in lib.rs and the 9
`paste_tests` for the routing table.

## 2026-07-14 - session persistence

`~/.moneyball/sessions/<id>.json`; CLI flags `-c`, `--resume <id>`, `--list`.
IDs are `mb-<UTC-timestamp>-<4-char random>` to dodge same-second collisions
when a user reruns quickly. See `crates/moneyball-core/src/session.rs`.

## 2026-07-14 - multi-line per-product brief

Single-line table didn't fit at narrow chat widths. Swapped to multi-line
per-product (`> {name}` / metric line / `L->Q` line). Production path uses
`format_brief_as_lines`; `BriefPlaceholder` cell kept as a future swap-in slot.
Render via `cargo run --example render_app -p moneyball-tui`.

## 2026-07-14 - Esc semantics

- Chat view: double-Esc primes + quits (matches codex/pi pattern).
- Multi-select (setup substep 1): Esc resets meta state and goes back to substep
  0 (token paste). The on-screen hint says "Esc=back" so the rule must match.
- Other setup substeps: Esc quits the wizard.

## 2026-07-14 - setup wizard shape

4 steps: workspace, Meta-connect (token paste + multi-select), products, goals.
The `target_rs_per_q` step is intentionally absent - it varies per-product and
per-industry (~Rs.100 e-commerce, ~Rs.2,500 real-estate). The advisor derives it
from observed performance. Config field exists but stays `None` after setup;
tunable later via direct JSON edit.

## Open questions / next

- `funnel` parity vs `mb.py funnel`. KILL_TABLE = {0: 3.0, 1: 4.7, 2: 6.3}.
- `/ask` LLM command is unwired - needs a real backend.
