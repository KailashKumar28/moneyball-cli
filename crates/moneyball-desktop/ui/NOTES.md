# Moneyball for Mac - UI notes

Status: built, tested, and every screenshot reviewed against the design (see "Verification").

## Scripts (pnpm, in this directory)

- `pnpm dev` - vite on :1420. Outside Tauri the fixture mock stands in for the shell.
- `pnpm build` - `tsc --noEmit` then `vite build` into `dist/` (relative asset paths).
- `pnpm typecheck`, `pnpm lint`, `pnpm test` (Playwright, webkit only), `pnpm check` (all four).
- First time: `pnpm install && pnpm exec playwright install webkit`.

## Structure

- `src/ipc/types.ts` - the contract's types; core shapes typed by hand, CRM fields nullable.
- `src/ipc/index.ts` - the only file that calls `invoke`; one function per command; every rejection becomes `AppError`.
- `src/ipc/fixtures/*.json` - hand-written fixtures (the shell's generated ones replace them).
- `src/ipc/fixtures.ts` - the drift alarm, outside the folder the shell overwrites. JSON imports type
  "refresh" as `string`, so `satisfies` checks against `Loose<T>` (literals widened): names, nesting,
  nullability and number-vs-string are checked, literal values are not. The file names it imports are
  mine; rename there if the shell's differ.
- `src/ipc/mock.ts` - dev/test only, dynamically imported behind `import.meta.env.DEV`, so not in `dist`.
- `src/App.tsx` (workspace, five screens switched by state, keys, health footer), `src/useRefresh.ts`
  (the one refresh, app-wide), `src/lib.tsx` (useAsync, formatters, shared components),
  `src/screens/*`, `src/styles.css` (the mock's CSS ported verbatim, app additions at the end).
- All screens stay mounted and are hidden, so a turn keeps streaming while you look at Funnel.

## How the IPC mock works

`mockIPC` from `@tauri-apps/api/mocks` answers every command from the fixtures. A `Channel` passed as
`onEvent` arrives in the handler as the object itself, so the mock calls its `onmessage` directly.
Flags come from the URL, `?mock=a,b`: `noworkspace`, `wsgone`, `degraded`, `nodata`, `fail`, `running`,
`noreport`, `turnactive`, `markers`, `nomodel`, `busy`, `manual`. Without `manual` a refresh or a turn
plays its fixture events on timers; with it the tests push events through `window.__mock.refresh(ev)`
and `window.__mock.turn(ev)`, and read `window.__mock.calls`. A text starting with `/` is refused
with `not_a_command`.

Tests: `tests/screens.spec.ts` is one test per screen state; each runs at 1200x800 and 900x600, light
and dark, runs the design's own wrap/clip/overflow check, and saves `shots/<state>-<theme>-<size>.png`
(names match `plans/desktop/design/shots`). `tests/behaviour.spec.ts` covers keys, re-reads, copy,
controls and the untrusted-markdown boundary. The clock is fixed, so elapsed times read 00:00.

## Contract changes (both confirmed by the lead)

1. `StepView.status` uses core's names: `done | partial | failed | skipped`, plus the UI-only
   `waiting | running`. IPC.md says `ok`; that was an error, `done` is right. `partial` gets the
   warning glyph ("no CRM", "4 of 5"), and so does any status the UI does not know.
2. Day numbers gain optional `source?: "meta" | "crm"` for the source dot. Without it no dot is drawn.

## Where the build differs from the mock, and why

- No drawn traffic lights (the native ones sit there).
- Refresh: a spinner and a ticking mm:ss for the step in flight, no in-step progress (contract).
- Today's data-health rows use core's labels and words ("Meta insights", "done"). The Day column shows
  the run's day only for a step that brought data in; the contract has no per-source day.
- Today caption is "Changes are against <day>." (the mock's "last comparable day" is a claim the UI cannot make).
- Brief subtitle is today's footer line ("Data Thu 08 Oct"); the contract has no worded window.
  Feasibility has no "6.3x today's spend" column: that is a computed figure.
- Brief/Funnel cells format core's raw numbers (grouping, decimals, %). That is the only number work in the UI.
- Advisor: no message times and no tool result summary ("5 products, 4ms"): neither is in ViewItem.
  Tool arguments are shown as their values. A tool row expands to its output. A Send button exists when idle.
- Report day options are formatted from the raw dates; "Month to date" sends the day of the month as `days`.
- UI-authored static copy (not from Rust): empty/loading lines, the Funnel no-CRM notice (the mock's
  sentence), column captions, "Turn stopped" / "Bug report filed", the composer status words.
- Cmd-O opens the picker. A failed pick shows the empty state with the error and a way back.
- `data-tauri-drag-region` is on each toolbar and the strip above the sidebar; title text has
  `pointer-events:none` so a press on it still drags.

- A tool call left `pending` when no turn is in flight (an interrupt cut it short) shows a ring and
  "stopped", not a spinner. Same idea on Refresh: after a run died or could not start, a step still
  marked running or waiting is a bare ring with no word.

## Verification

- `pnpm check`: tsc, eslint, 40 Playwright tests (webkit), `vite build` - all clean.
- `shots/` holds 108 screenshots: 27 states x light/dark x 1200x800/900x600. The 11 design states
  were compared with `plans/desktop/design/shots` at full size (at least one variant each, both
  sizes and both themes covered); every one of the 108 was then looked at on contact sheets.
- Fixed from that review: the Brief feasibility grid, a double "Turn stopped" marker (StrictMode
  ran a state updater twice), the notice inside the empty state (wrong margin and color), spinners
  left running on dead runs and cut-short tool calls, and the report-none shot showing a report.
- Known rendering differences from the design shots, not defects: WebKit draws New York slightly
  heavier than the Chromium the mock was shot in, and rows sit about 1px differently.
- Not verified here: the real WKWebView inside Tauri, native window drag, and real IPC timing.
