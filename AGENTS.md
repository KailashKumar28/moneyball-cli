# AGENTS.md - moneyball-cli
`moneyball` is a Rust CLI for Meta-ads portfolios: TUI REPL + slash-commands +
sub-commands + daily advisor. TUI is the full management surface; no web UI.
Users bring their own Meta token + CRM; moneyball reads fetcher output via a
documented snapshot schema. **Never call Meta or CRM APIs in moneyball core.**

## Hard-won lessons from session 1
- **Iterate, don't sprint.** One slice at a time: implement + test + show the user actual output + get sign-off BEFORE moving on.
- **UI placeholders are bugs.** Every wizard step is a real form with clear instructions.
- **Defaults adapt to where the user is.** Workspace = `<cwd>/moneyball-data`, never `~/moneyball-data`. Auto-create user-confirmed paths.
- **Always render via `TestBackend` before installing.** Catches UI issues before the user screenshots them.
- **First-run TUI NEVER errors.** `main.rs` uses `resolve_optional()`; sub-commands that strictly need a workspace (`brief`) re-resolve strict.
- **One fix at a time, then wait for sign-off.**

## Architecture (locked 2026-07-13)
Rust workspace, edition 2021, MSRV 1.96. Headless-core split: `moneyball-core`
= logic (snapshot, brief, funnel, advisor, ledger writes); `moneyball-tui` =
thin ratatui front-end; `moneyball` binary = `clap` dispatcher. Same code
path; TUI subscribes to typed event deltas. Only writes: `ledger.jsonl` and
`runs/<date>/`. `MB_AGENT=1` = agent mode. Tests = `cargo test`, hermetic.

## First-run wizard
1. Workspace - `<cwd>/moneyball-data` default, auto-create on Enter. 2. Add
products - `Name AdAccount` Enter, or `demo` for Fincity, blank to finish. 3.
Goals per product - blank Enter accepts defaults of 10. 4. Target Rs/qualified
- default 2500, blank accepts.

**Next:** insert "Connect Meta" step - paste a long-lived Marketing API token,
`GET /me/adaccounts` validates + lists, user picks accounts, token saved to
OS keychain via `keyring`. **Optional** (skip keeps manual entry). OAuth browser
flow is future - needs a registered Meta App.

## Code style (Rust)
- Snapshot numeric fields are `String` (Meta returns strings); use `parse_f64`/`parse_u64` helpers in `AdsDailyRow`.
- M-Leads: mirror `mb.py:leads_from_actions` exactly - first-match canonical list, then "lead"-substring fallback. See `count_m_leads` in `brief.rs`.
- CRM tickets can be flat array OR `{campaign_id: {tickets: [...]}}` - normalize via `for_each_crm_ticket`.
- Window math: `d0 = yesterday - ndays + 1`, `d1 = yesterday`; snap day excluded. IST epoch helper.
- Errors: `thiserror` in `error.rs`; `anyhow` only at binary edge. No `unwrap()` in library code.
- Hand-rolled table formatter in `brief.rs:format_brief_table` (parity with `mb.py:_tab`).

## Where new things go
- Sub-command `foo` -> clap variant in `crates/moneyball/src/main.rs` + math in `crates/moneyball-core/src/foo.rs`. Don't add to TUI `submit()` until headless parity-tested.
- TUI view -> new `View` variant with `render_*` + `handle_*_key`.
- Render harness -> `crates/moneyball-tui/examples/render_*.rs` via `TestBackend`.
- Test -> `crates/moneyball-core/tests/` with hermetic fixture under `fixtures/snap/<date>/`.

## Definition of done
1. `cargo build` clean. 2. `cargo test` green. 3. Render via TestBackend and confirm. 4. Run vs `2026-07-13`, parity with `python3 pipeline/mb.py <cmd>`. 5. **Show user actual stdout.** 6. `cargo install --path crates/moneyball --locked --quiet`.

## Don'ts
- Do NOT import from `pipeline/mb.py` or any third-party pipeline. Reimplement.
- Do NOT call Meta or CRM APIs from moneyball-core. Reads snapshots only.
- Do NOT write anywhere except `ledger.jsonl` and `runs/<date>/`.
- Do NOT fix "Stattic Ad" Meta typo (breaks LeadZump ad-id join). Do NOT bucket CRM by `created_at` for short windows - use `delivery_time`.
- Do NOT use `AppConfig::resolve()` in `main.rs`. Use `resolve_optional()`; re-resolve strict in sub-commands.
- Do NOT trust `openclaw/openclaw` or `NousResearch/hermes-agent` as design refs (implausible stars). Use Claude Code / gemini-cli / pi or Codex instead.

## Current state
Built: `brief` (parity-verified vs `mb.py scoreboard`), TUI with brief view +
slash-completion + Esc quit, first-run wizard. Installed at
`~/.cargo/bin/moneyball`. Next: `funnel` (parity), then Meta-connect wizard.