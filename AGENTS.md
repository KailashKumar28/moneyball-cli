# AGENTS.md - moneyball-cli

Read-only Meta-ads advisor. The READ/analysis path (brief, funnel, advisor
math) never touches the network; the only network modules are `meta.rs`
(discovery), `fetch.rs` (explicit snapshot pull), `llm.rs` (model calls),
`crm/fetch.rs` (explicit CRM pull) - and nothing ever writes to Meta or the
CRM. Analysis reads snapshots via the documented schema, whether written by
`/fetch`, `crm fetch`, or an external pipeline. TUI is the management
surface; no web UI.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the BINDING structure & style
contract: crate layout, module/size rules, anti-verbosity rules, TUI/LLM
patterns, and the definition-of-done gates. Read it before writing code.
See [HANDOFF.md](HANDOFF.md) for recent decisions (prune, don't grow).

## Definition of done

1. `cargo build --workspace` clean.
2. `cargo test --workspace` green.
3. `cargo clippy --workspace --all-targets -- -D warnings` clean.
4. Render via TestBackend (`cargo run --example render_*`); confirm visually.
5. Headless CLI parity vs `python3 pipeline/mb.py <cmd>`.
6. **Show user actual stdout.**
7. `cargo install --path crates/moneyball --locked --quiet`.

## Hard-won lessons

- **Iterate, don't sprint.** One slice = one bug fix, one feature, or one QA
  pass. Show user actual output + get sign-off before starting the next.
- **UI placeholders are bugs.** Setup wizard steps must each render a
  `prompt_line(...)` with explicit instructions (see `render_step_*` in lib.rs).
  No empty body panels - verify with `render_views` harness before commit.
- **Defaults adapt to where the user is.** Workspace = `<cwd>/moneyball-data`,
  never `~/moneyball-data`. Auto-create user-confirmed paths.
- **First-run TUI NEVER errors.** `main.rs` uses `resolve_optional()`; sub-
  commands that strictly need a workspace (`brief`) re-resolve strict.
- **ASCII only in TUI output.** Replace multibyte with ASCII (`U+25B8` -> `>`,
  `U+20B9` -> `Rs.`). Some terminal fonts can't render multibyte.

## Where new things go

- Sub-command `foo` -> clap variant in `crates/moneyball/src/main.rs` + math in
  `crates/moneyball-core/src/foo.rs`. Don't add to TUI `submit()` until headless
  parity-tested.
- TUI view -> new `View` variant with `render_*` + `handle_*_key`.
- Render harness -> `crates/moneyball-tui/examples/render_*.rs` via
  `TestBackend`.
- Test -> `crates/moneyball-core/tests/` with hermetic fixture under
  `fixtures/snap/<date>/`.

## Don'ts

- Do NOT import from `pipeline/mb.py` or any third-party pipeline. Reimplement.
- Do NOT write anywhere except `ledger.jsonl` and `runs/<date>/`.
- Do NOT fix "Stattic Ad" Meta typo (breaks LeadZump ad-id join). Do NOT bucket
  CRM by `created_at` for short windows - use `delivery_time`.
- Do NOT use `AppConfig::resolve()` in `main.rs`. Use `resolve_optional()`;
  re-resolve strict in sub-commands.
- Do NOT trust `openclaw/openclaw` or `NousResearch/hermes-agent` as design refs
  (implausible stars). Use Claude Code / gemini-cli / pi or Codex.
