# Moneyball for Mac - plan

Decided 2026-11-12: Tauri 2, daily-driver v1, signed + notarized DMG.
Research behind every choice here (read before changing one):
[research-core-seam.md](research-core-seam.md),
[research-references.md](research-references.md),
[research-tauri.md](research-tauri.md).

## What the research changed

1. The Tauri glue is the small part. Core holds the math and the I/O, but
   every worker thread, the tool executor, the system prompts and almost
   every user-facing sentence live in `moneyball-tui`. A second front-end
   built today would fork all of it. So the first wave is extraction into
   core, with the TUI switched over and behaving exactly as before.
2. ARCHITECTURE 6b names an `Op` enum that does not exist in code. We fix
   the doc to say what is true rather than build the enum.
3. Seam shape: pi's (direct methods + one event stream), not codex's
   JSON-RPC app-server. codex's own in-process host says it keeps the
   envelope only to match its stdio/ws clients; we have none.
4. From Finder the app has cwd `/` and no shell env, so workspace
   resolution lands on `/moneyball-data`. The app must hold an explicit
   workspace and remember it.
5. Setup lives only in the TUI (1,824 lines) and is out of v1. v1 opens an
   existing workspace; with none, it says so and names the terminal command.

## Shape

```
moneyball-core      + advisor session driver, tool executor, prompts,
                      landing/refresh words, owned refresh events,
                      view projection, Serialize on what crosses IPC
moneyball-tui       styling + key handling only for the moved pieces
moneyball-desktop   NEW. Tauri 2 shell: commands in, Channel events out.
  src/              Rust (under the 400-line cap, arch_contract scans it)
  ui/               React + TypeScript + Vite, plain CSS tokens
moneyball           unchanged
```

Dependency rule becomes: `moneyball -> moneyball-tui -> moneyball-core`
and `moneyball-desktop -> moneyball-core`. Desktop never depends on the
TUI crate; core never gains a tauri or ratatui dependency.

- IPC in: one `#[tauri::command]` per operation, returning immediately.
- IPC out: `tauri::ipc::Channel<T>` per turn / per refresh, fed by a
  forwarder thread over core's std mpsc; text deltas coalesced ~30ms.
  Core stays blocking threads, no tokio.
- Live session state stays in Rust (`tauri::State`), so a webview reload
  re-reads a snapshot and resubscribes.
- One projection `&[Item] -> Vec<ViewItem>` in core (tool call + output
  joined by call_id) used by live stream, reopen, and the TUI replay.
- TypeScript types are generated from the Rust types and pinned by a test,
  like docs/schemas.
- The webview has no network: CSP `connect-src ipc:` only. The report is
  shown in `<iframe sandbox srcdoc>` (it has no scripts), with "open in
  browser" beside it.
- Secrets never cross IPC. No command returns a token or key.
- The advisor prompt gets the desktop's own command list (none in v1) and
  its own rendering rule (markdown renders here). A message starting with
  `/` is never sent to the model.
- Fixes that are terminal commands are shown as "In Terminal:" with a copy
  button until the desktop grows that surface itself.

## v1 surfaces and commands

| Surface | Commands | Events |
|---|---|---|
| Workspace | `workspace_current`, `workspace_pick` (folder dialog), `workspace_open(path)` | - |
| Landing (data health) | `landing` | - |
| Brief | `brief` | - |
| Funnel | `funnel(product, days, by)` | - |
| Refresh | `refresh_start(on_event)`, `refresh_cancel` | Begin, Done(step), Finished(run), CouldNotStart |
| Report | `report_for(date?)`, `report_generate(days)`, `report_open_in_browser` | - |
| Advisor | `sessions_list`, `session_open(id)`, `session_new`, `turn_start(text, on_event)`, `turn_cancel` | Delta, MessageDone, ToolBegin, ToolEnd, TurnEnded(status) |

Not in v1 (stay in the TUI/CLI): setup, connect, schedule, status history,
bug reports, diagnose view, ledger view.

## Waves

Each helper works in its own git worktree on its own branch off `desktop`;
nothing touches `master` and nothing is pushed until you say so.

**Wave 1 - extraction and design (parallel, 4 helpers)**
- `core-agent`: session driver, tool executor, prompts, app state block,
  typed tool turn, view projection, session previews, Serialize on Ev.
  (seam doc section 2: A B C C2 C3 D E F H J2)
- `core-words`: landing sentences, day numbers, fix wording, finished-run
  wording as unstyled structured output in core. (L M M2 O2)
- `core-data`: owned refresh events + worker, brief loader bundle,
  Serialize on brief/funnel/report/health types, serializable error view,
  explicit workspace open + remembered last workspace. (N Q + section 4, 6)
- `design`: static HTML mock of every v1 screen, light and dark, for your
  sign-off before any UI code.
- Gate: TUI render examples byte-identical before/after; fmt, clippy -D
  warnings, tests green; then merged into `desktop` and `cargo install`ed.

**Wave 2 - the app (parallel, 2 helpers, after wave 1 merges)**
- `shell`: the Tauri crate - commands, state, forwarders, window chrome,
  single instance, CSP, arch_contract extended to the fourth crate.
- `ui`: the frontend, built against mocked IPC with Playwright from the
  signed-off design.

**Wave 3 - integration (me + a reviewer helper)**
- Wire real IPC, drive the real app end to end against a real workspace,
  screenshot every screen, fix what looks off.
- Independent review of the whole branch.
- AGENTS.md / ARCHITECTURE.md amended (topology, I/O exception for
  WebKit's own dirs under ~/Library, 6b seam truth, desktop definition of
  done); TODO.md updated.

**Wave 4 - release**
- `scripts/release-desktop.sh`: build, sign, notarize the app, notarize
  and staple the DMG (Tauri leaves the DMG un-notarized), then assert with
  codesign / spctl / stapler so a silent "skipping notarization" fails.
- Blocked on you: Apple Developer Program membership, a Developer ID
  Application certificate in the login keychain, Team ID, and an App Store
  Connect API key (Issuer ID, Key ID, .p8). Until then the script builds
  an ad-hoc signed app that runs on this Mac only.

## Known ceilings in v1

- No first-run setup in the app (use `moneyball` in a terminal once).
- Model keys or crm.toml values that come from shell env vars are not
  visible to a Finder-launched app; keys stored via connect are.
- TUI and desktop resuming the same session at the same moment can
  interleave lines; single-instance guards desktop against itself only.
- Refresh cancel takes effect between steps, as in the TUI.
