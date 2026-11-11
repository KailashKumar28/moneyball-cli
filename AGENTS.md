# AGENTS.md - moneyball-cli

Read-only Meta-ads advisor. The READ/analysis path (brief, funnel, advisor
math) never touches the network; network code lives in exactly four core
modules (canonical list: ARCHITECTURE.md §1, enforced by the arch_contract
test) - and nothing ever writes to Meta or the CRM.
Analysis reads snapshots via the documented schema, whether written by
`/refresh`, `crm fetch`, or an external pipeline. The TUI is the management
surface. The Mac app (`crates/moneyball-desktop`, Tauri) is a second
front-end over the same core for the daily surfaces (today, brief, funnel,
refresh, report, advisor); its webview has no network of its own. No
hosted web UI.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the BINDING structure & style
contract: crate layout, module/size rules, anti-verbosity rules, TUI/LLM
patterns, and the definition-of-done gates. Read it before writing code.
See [HANDOFF.md](HANDOFF.md) for recent decisions (prune, don't grow).
See [TODO.md](TODO.md) for the project backlog - pick work from the top,
check items off in the same commit that ships them.

## Definition of done

1. `cargo build --workspace` clean.
2. `cargo test --workspace` green.
3. `cargo clippy --workspace --all-targets -- -D warnings` clean.
4. Render via TestBackend (`cargo run --example render_*`); confirm visually.
5. Headless CLI parity vs `python3 pipeline/mb.py <cmd>`.
6. **Show user actual stdout.**
7. `cargo install --path crates/moneyball --locked --quiet`.
8. Desktop changes: `pnpm check` in `crates/moneyball-desktop/ui`, then the
   real-app run in `crates/moneyball-desktop/e2e` (its README), and read
   its screenshots. A UI change seen only against the mocked IPC isn't
   shipped.

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
- Anything both front-ends say or do (a sentence, a loader, a worker, a
  prompt) -> core, unstyled; the TUI and the desktop only present it.
  Steps the user takes are named through `FrontEnd.say` / `words::Command`
  so each front-end words them its own way.
- Desktop surface -> a command file in `crates/moneyball-desktop/src/`, its
  shape in `plans/desktop/IPC.md` and the pinned fixtures, then the screen
  in `ui/src/screens/`. The UI never composes product wording or computes
  a figure.

## Don'ts

- Do NOT import from `pipeline/mb.py` or any third-party pipeline. Reimplement.
- Do NOT write outside `<workspace>/.moneyball/` and `~/.moneyball/`.
  Fetch writes only `history/snap/<date>/` and `history/assets/`; report
  generation writes only `reports/<date>/`; never mutate a prior date's
  snapshot from the report path. (`~/.moneyball/bug-reports/` = user bug
  reports; `<workspace>/.moneyball/reports/` = creative reports - two
  different things.) The one exception: `/schedule` (and `moneyball
  schedule --at`) writes `~/Library/LaunchAgents/ai.moneyball.refresh.<id>.plist`,
  only after the user confirms, and `--off` / `/schedule off` removes it
  (core `schedule.rs` is the only writer). The Mac app adds nothing of its
  own outside those two roots; WebKit keeps its caches under `~/Library`
  for the bundle id, which we do not control.
- Do NOT let the desktop tell a user (on screen or through the advisor) to
  type a slash command, and do NOT return a secret over IPC.
- Do NOT fix "Stattic Ad" Meta typo (breaks LeadZump ad-id join). Do NOT bucket
  CRM by `created_at` for short windows - use `delivery_time`.
- Do NOT use `AppConfig::resolve()` in `main.rs`. Use `resolve_optional()`;
  re-resolve strict in sub-commands.
- Do NOT trust `openclaw/openclaw` or `NousResearch/hermes-agent` as design refs
  (implausible stars). Use Claude Code / gemini-cli / pi or Codex.
