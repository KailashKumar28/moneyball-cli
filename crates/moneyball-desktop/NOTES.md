# moneyball-desktop: shell notes

The Rust half of Moneyball for Mac: a Tauri 2 shell over `moneyball-core`.
The contract it implements is `plans/desktop/IPC.md`; where this crate and
that file (or the UI's `types.ts`) differ, the difference is listed under
"Contract changes" below.

## Run and build

All from `crates/moneyball-desktop/`. The Tauri CLI is expected as a dev
dependency of `ui/` (`pnpm add -D @tauri-apps/cli@^2`).

| What | Command |
|---|---|
| Dev (vite + the app, restarts on Rust edits) | `./ui/node_modules/.bin/tauri dev` |
| Release bundle (`.app` and `.dmg`) | `./ui/node_modules/.bin/tauri build --bundles app,dmg` (signed release: `scripts/release-desktop.sh`) |
| Rust only | `cargo build -p moneyball-desktop` |
| Tests | `cargo test -p moneyball-desktop` |
| End to end, the real app (`e2e/README.md`) | `node e2e/run.mjs` |
| Regenerate the IPC fixtures | `MB_UPDATE_FIXTURES=1 cargo test -p moneyball-desktop fixtures` |
| Regenerate the icon set | `python3 icons/make_icon.py` |

- Run the CLI from this directory, not through `pnpm --dir ui`: it looks
  for `tauri.conf.json` under the current directory.
- `cargo build` needs `ui/dist/index.html` (Tauri embeds it at compile
  time). `build.rs` writes a stand-in page when it is missing, so a fresh
  checkout can run `cargo test --workspace`; `pnpm build` replaces it.
  `ui/dist` and `gen/schemas` are ignored by this crate's `.gitignore`.
- A plain `cargo build` binary loads `devUrl` (localhost:5173), not the
  embedded page. Only `tauri build` produces an app that serves `ui/dist`.
- `devUrl` and `devCsp` name port 1420, the one `ui/vite.config.ts` pins
  (they said 5173, which nothing listens on). Changed by reading; `tauri
  dev` itself has not been run.
- The real app is driven end to end by `e2e/` (WebdriverIO over a
  WebDriver server compiled in by the `e2e` cargo feature only). A normal
  build has neither the feature, the plugins, `withGlobalTauri` nor any
  extra permission: all of that lives in `e2e/tauri.e2e.conf.json`.

## Layout

| File | Concern |
|---|---|
| `lib.rs` | table of contents, plugins, command registration |
| `state.rs` | `AppState` |
| `action.rs` | core `Command` to `Action` (`translate`) and its twin in words (`say`) |
| `error.rs` | `AppError` |
| `front_end.rs` | the advisor's `FrontEnd`, system prompt, CRM note |
| `workspace.rs`, `today.rs`, `brief.rs`, `funnel.rs`, `report.rs`, `session.rs`, `clipboard.rs` | one screen's commands each |
| `run_view.rs` | `StepView` / `RunView`, shared by Today and Refresh |
| `refresh.rs`, `refresh_end.rs` | refresh commands, forwarder, the finished event |
| `turn.rs` | turn commands, forwarder with delta coalescing |
| `fixtures/` (tests only) | the pinned IPC fixtures and the no-slash-command check |

Every command is a thin `#[tauri::command]` over a plain function that
takes `&AppConfig` (and state where needed). The tests and the fixtures
call those plain functions, so no Tauri runtime is needed to test.

## State model

`AppState` in `tauri::State`. The webview holds nothing: after a reload it
re-reads `workspace_current`, `refresh_state` and `session_current`.

- `workspace: Mutex<Option<AppConfig>>`. Set by `workspace_current` (which
  re-opens the remembered folder from disk on every call) and
  `workspace_pick`. Every other command clones it or returns
  `AppError { kind: "no_workspace" }`.
- `session: Mutex<Option<Arc<Mutex<Session>>>>`. The `Arc` is shared with
  the forwarder thread of the turn in flight. Opening a different
  workspace interrupts a running turn and drops the session; the forwarder
  still holds it and finishes writing its file.
- `refresh: Arc<Mutex<RefreshRun>>`: running flag, the day, the four step
  rows, start time, cancel flag. Shared with the refresh forwarder. It
  keeps the last run's rows after the run ends.

Threads. Core spawns the worker (refresh or turn). The command spawns one
forwarder thread that drains core's `Receiver` and returns at once.

- Turn forwarder: `Session::apply` for every event, then send; text deltas
  gather for 30 ms and go out as one `assistant_delta`; `stream_closed` at
  the end; `died` if no final event came. The session is free before the
  final event is sent, so the UI can start the next turn from its handler.
- Refresh forwarder: rewords each event, updates `RefreshRun`, sends. The
  run is marked over before its last event is sent.
- A failed channel send means the webview is gone. Sending stops; the turn
  or refresh carries on and state and the session file stay complete.
- Commands that touch disk are `#[tauri::command(async)]` so they run off
  the main thread. The folder dialog blocks that worker, not the UI.

Safety.

- No command returns or logs a token or key. The Meta token is read in
  `refresh::start` and handed to core only.
- `report_open_in_browser` canonicalizes the path and the workspace's
  `.moneyball/reports/`, and opens only an `.html` file under it.
- The opener and the folder dialog are called from Rust. The capability
  file grants `core:default` and `core:window:allow-start-dragging` only.
- Session ids from the webview are limited to `[A-Za-z0-9-]`; report dates
  must parse as `YYYY-MM-DD` (both name files).
- Clipboard is `/usr/bin/pbcopy` (macOS only, no plugin, no permission).
- CSP is the research doc's, `connect-src ipc: http://ipc.localhost` only.
  `dangerousDisableAssetCspModification: ["style-src"]` is set so Tauri
  adds no style nonce: a nonce would switch off `'unsafe-inline'` and blank
  the report's inline `<style>` in the `srcdoc` iframe. Checked in the
  real WKWebView by `e2e/` (the report renders styled under this CSP).
  `devCsp` adds the vite origin and inline scripts; untested.

## Contract changes

Changes the lead already confirmed:

1. `StepView.status` is `done | partial | failed | skipped | waiting |
   running`. The UI's `types.ts` on `desktop-ui` still says `ok` where this
   crate sends `done`.
2. Day numbers carry `source: "meta" | "crm"`. Core does not provide it;
   `today.rs` derives it from the label and omits it for an unknown label.

Mine, each the smallest choice I could see:

3. `Action` refresh has `force: boolean`. Core's landing asks for
   `/refresh --force` when a pull missed products; without the flag the
   button would run a refresh that reuses the incomplete pull. The UI must
   pass `action.force` to `refresh_start`. Not in `types.ts`.
4. `session_current` and `session_new` do not create a session file. The
   file is created by the first `turn_start`, so `id` is `null` until then.
   Core lists empty sessions, so creating one per launch would fill the
   list with blanks. The UI's `session_new.json` had a non-null id.
5. `workspace_pick` returns `busy` while a refresh runs (its events belong
   to the old workspace's screen). Checked before the dialog opens.
6. Dates. `report_days`, the `date` arguments and `ReportView.date` are
   snapshot dates (what core keys reports by). `ReportView.day` and
   `RunView.date` / `day` are the data day, one day earlier.
7. Error kinds beyond IPC.md: `no_workspace` (nothing open),
   `no_workspace_config` (folder has no config; fix is the terminal),
   `could_not_start` (inside the refresh event), `refused` (report path),
   `session` (cannot open), `empty` (blank message), `io`, and core's own
   (`snapshot`, `config`, `crm_auth`, ...). The UI's fixture used
   `snapshot_unreadable`, `no_meta_token` and fixes like `moneyball connect
   model`; none of those exist. The UI does not branch on error kinds today.
8. `RunView.gaps` for a failed run is "stopped at step N" (the TUI's
   header); IPC.md only describes the degraded case.
9. `BriefView.cpl_order` and `staleness` are whole sentences:
   "cheapest Meta lead first: ..." and core's warning without its "!! "
   terminal mark.
10. `sessions_list` returns `[]` when the sessions folder cannot be read,
    not an error.
11. `StepView` has no field for the "from today's earlier pull" note; the
    UI has `reused: true` and must word it itself or the contract needs a
    `note`.
12. Fixture `fixtures/snap/` in IPC.md does not exist with usable data
    (core's has one `creatives.json`). The demo data lives in this crate's
    `fixtures/` (config + two snapshots, generated once, no real names).

## Wording, for the lead

- Money is "Rs." everywhere, because core's values are. The design asks
  for the rupee sign; that is a core change or a UI display rule.
- `translate` turns `/setup` and `/setup meta` into the command
  `moneyball` with "then type /setup meta there" in `does`, since the
  binary has no `setup` subcommand. That text is for the terminal app, on
  screen only. `say` (what the advisor reads) never names a slash command:
  tested over every fix string, the full system prompt and every tool
  output for the demo, with and without CRM (`fixtures/wording.rs`).
- `say("/brief")` is "the Brief screen". Core's sentence around it says it
  "self-heals"; the Brief screen does not pull data. Core wording issue.
- `front_end::crm_note` loads the snapshot a second time to call
  `CrmHealth::note_for` (only when there is a note). If `brief::Loaded`
  carried the `CrmHealth`, that read would go.
- `turn.rs` repeats core's private `model_for` check so a refused turn
  does not create a session file. Exposing it from core would remove the
  copy.
- Today's footer lines and the feasibility sentence are worded here
  (`today.rs`, `brief.rs`), following the TUI's context bar and the mock.

## Fixtures

`ui/src/ipc/fixtures/*.json`, 21 files, with the UI helper's file names
(`fixtures.ts` on `desktop-ui`). Built by `src/fixtures/` from a temp copy
of `fixtures/` through the command bodies. Refresh and turn streams are
core events fed by hand to the real forwarders, since a real run needs the
network. The clock is held at 2026-10-09: the health reading's `today`,
`state` and `refreshed_at` are set in the test, and `staleness`,
`modified_at`, the report's generated-at stamp and the temp path are
pinned. `errors.json` has one extra key, `no_workspace`.

Not checked: `tsc` against these files. `types.ts` is not on this branch;
by reading, the differences are items 1 and 3 above.
