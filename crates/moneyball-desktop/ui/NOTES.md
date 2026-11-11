# Moneyball for Mac - UI notes

Status: integrated with the shell's generated fixtures; built, tested, and every screen state looked at
with the real demo data (see "Verification"). Open points for the lead are under "For the lead".

## Scripts (pnpm, in this directory)

- `pnpm dev` - vite on :1420. Outside Tauri the fixture mock stands in for the shell.
- `pnpm build` - `tsc --noEmit` then `vite build` into `dist/` (relative asset paths).
- `pnpm typecheck`, `pnpm lint`, `pnpm test` (Playwright, webkit only), `pnpm check` (all four).
- First time: `pnpm install && pnpm exec playwright install webkit`.

## Structure

- `src/ipc/types.ts` - the contract's types, matching what the shell sends; CRM fields nullable.
- `src/ipc/index.ts` - the only file that calls `invoke`; one function per command; every rejection becomes `AppError`.
- `src/ipc/fixtures/*.json` - the shell's generated fixtures (Rust command bodies over the demo data,
  pinned by a Rust test). Never edited here.
- `src/ipc/fixtures.ts` - the drift alarm. Each fixture goes through `fixture<T>()(json)`, which fails
  `tsc` when (a) a field the type requires is missing, misnested, or differs in nullability or
  number-vs-string (`Loose<T>`), or (b) the JSON has a field the type lacks, at any depth, named in the
  error as a path like `.day.numbers[].source` (`Extra`). `errors.json` is keyed by its own keys, so a
  dropped error fails too. What tsc cannot see: literal values (a JSON import types "done" as `string`),
  tuple lengths, and whether a nullable or optional field is ever null. `tests/fixtures.spec.ts` covers
  the literals at run time: every status, tint, kind, type, trigger and source in every fixture must be
  a member of the union in `types.ts`.
- `src/ipc/mock.ts` - dev/test only, dynamically imported behind `import.meta.env.DEV`, so not in `dist`.
- `src/App.tsx` (workspace, five screens switched by state, keys, health footer), `src/useRefresh.ts`
  (the one refresh, app-wide), `src/lib.tsx` (useAsync, formatters, shared components),
  `src/screens/*`, `src/styles.css` (the mock's CSS ported verbatim, app additions at the end).
- All screens stay mounted and are hidden, so a turn keeps streaming while you look at Funnel.

## How the IPC mock works

`mockIPC` from `@tauri-apps/api/mocks` answers every command from the fixtures. A `Channel` passed as
`onEvent` arrives in the handler as the object itself, so the mock calls its `onmessage` directly.
Flags come from the URL, `?mock=a,b`: `noworkspace`, `wsgone`, `degraded`, `fail`, `running`,
`noreport`, `turnactive`, `markers`, `nomodel`, `busy`, `manual`. The mock serves generated fixtures
only. A test that needs another state passes `open(page, flags, replies)`: a fixed reply per command,
set on `window.__replies` before the page loads. The mock keeps the shell's session rule: a new session
has `id: null` until `turn_start`. Without `manual` a refresh or a turn
plays its fixture events on timers; with it the tests push events through `window.__mock.refresh(ev)`
and `window.__mock.turn(ev)`, and read `window.__mock.calls`. A text starting with `/` is refused
with `not_a_command`.

Tests: `tests/screens.spec.ts` is one test per screen state; each runs at 1200x800 and 900x600, light
and dark, runs the layout check, and saves `shots/<state>-<theme>-<size>.png` (names match
`plans/desktop/design/shots`). `tests/behaviour.spec.ts` covers keys, re-reads, copy, controls, the
session id rule and the untrusted-markdown boundary. `tests/fixtures.spec.ts` is the literal check.
The clock is fixed, so elapsed times read 00:00.

Expectations are derived from the fixtures (`tests/fixtures.ts` reads the JSON): row counts, words,
commands, error sentences, ids and dates are never typed into a test, so regenerating the fixtures
does not break one. Only UI-authored copy is asserted as a literal.

The layout check is the design's (no label, cell or button wraps, clips or leaves the window), relaxed
where real data needs a rule instead: sentences from core may wrap (`.cap`, `.why`, the step result
cell, the run summary, the feasibility line) and a name wider than its column is cut with an ellipsis
(`td.nm`, the sidebar's workspace name and path), its `title` holding the whole text.

### Hand-written fixtures (tests/fixtures.ts only, typed against types.ts)

States the generated set does not contain. Their words are modelled on core's but are not core's output.

| Name | State | Why it is needed |
|---|---|---|
| `funnelAllVerdicts` | funnel-verdicts | generated funnels have only keep, kill and no CRM; adds "wait: immature", "wait: low spend", "wait: learning", a zero beside an unknown, and an 80-character ad set name |
| `noData` | today-, brief-, funnel-, report-, advisor-, refresh-nodata | no generated fixture is a workspace that never pulled |
| `refreshCrmFailed` | refresh-crm-failed | no generated run has a `failed` step, a step `fix` beside a failure sentence, or a long `detail` |
| `refreshReused`, `todayReused` | refresh-reused | no generated step has `reused: true` |

## Contract changes

1. `StepView.status` uses core's names: `done | partial | failed | skipped`, plus the UI-only
   `waiting | running`. IPC.md says `ok`; that was an error, `done` is right. `partial` gets the
   warning glyph ("no CRM", "4 of 5"), and so does any status the UI does not know.
2. Day numbers gain optional `source?: "meta" | "crm"` for the source dot. Without it no dot is drawn.

Both confirmed by the lead. From the shell's NOTES ("Contract changes" 3 to 12), as built here:

3. A refresh `Action` carries `force`; the UI passes it to `refresh_start`.
4. A session's `id` is null until its first turn. The UI asks `session_current` once after the first
   `turn_start` of such a session resolves, to learn the id; the session list then shows and marks it
   while the answer streams.
5. Error kinds are open strings. The UI shows `reason` and `fix` and never branches on `kind`.
6. `report_days`, the `date` arguments and `ReportView.date` are snapshot dates; `ReportView.day` and
   `RunView.day` word the data day. See "For the lead" 1.
7. `StepView.reused` has no sentence in the contract; the UI words it "from today's earlier pull"
   (core's own note names `--force`, a flag this app does not have). Shown on Refresh and in Today's
   Data health table.

## Where the build differs from the mock, and why

- No drawn traffic lights (the native ones sit there).
- Refresh: a spinner and a ticking mm:ss for the step in flight, no in-step progress (contract).
- Money is "Rs." wherever the UI formats a figure or heads a column, because every sentence and day
  number from core says "Rs."; the design's rupee sign is one constant away (`RS` in `lib.tsx`).
- Sentences from core that start in lower case (an error's reason, "cheapest Meta lead first: ...",
  "no CRM in this report") get a capital first letter by CSS, as core's labels already did.
- Long text: a fix's "does" sits beside its command when there is room and under it when not; a failed
  step's sentence wraps in its cell; names are cut with an ellipsis (rules at the end of `styles.css`).
- Report's empty state says "this day" and does not name it (see "For the lead" 1). The iframe's title
  is "Report for <day>", not the report's summary, which is a page of text.
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

## For the lead

Things the Rust side returns that the UI shows as given, without working around them.

1. Report days are snapshot dates. The picker formats what `report_days` returns, so it reads
   "Fri 09 Oct" for the report that says "Thu 08 Oct" inside, and that Today calls Thu 08 Oct. The UI
   does not subtract a day (that is core's rule, `Health::data_day`). Suggested: `report_days` returns
   `{ date, day }[]`. Same cause: "Month to date" sends the day of the month of the date it holds, 9
   for a data day of the 8th, one day too many. It needs the data date, or a named window that Rust
   resolves. No test asserts the month-to-date number for that reason.
2. Money. Core says "Rs." in every value and sentence; the design asks for the rupee sign, and the
   report's own HTML uses it, so the Report screen and the rest of the app differ. The UI follows core.
3. `reused` needs a `note` field (see "Contract changes" 7), and a generated fixture that has one.
4. The generated fixtures have no: failed step, reused step, "wait" verdict, no-data workspace, Brief
   without CRM (`crm_note`, null columns), `report_missing`, `staleness`, `fault`, `partial` or `stale`
   health, or `refresh_start`'s own `busy` error (the mock answers with `refresh_busy`, whose sentence
   is about Generate). The first four are hand-written here; the rest are untested against real shapes.
5. `report.json`'s HTML carries "pulled 12 Nov, 04:01 am IST": a wall-clock stamp that is not pinned,
   so the fixture changes whenever it is regenerated.
6. Error wording, shown as sent: the `snapshot` error is the parser's own line with no fix ("snapshot
   2026-10-09/ads_daily: invalid JSON: key must be a string at line 1 column 3"); the `failed` turn
   hint is appended with no sentence break ("... 200000 maximum Start a new session to continue.").
7. Counts read "59 tickets, 1 pages" (core's `counts_detail` does not do the singular).
8. `Today.health.run.steps[].fix` is core's raw "/connect crm". The UI never draws `health.run`, but
   a slash command does cross the IPC there.
9. `workspace.display_path` in the fixture is the absolute path, not the "~" form IPC.md describes
   (perhaps only because the fixture's home differs). The sidebar cuts a long path with an ellipsis.
10. A step's own `fix` is not drawn in its row: the same action arrives in `fixes` with the finished
    event. During a run a failed step therefore shows its fix only once the run ends.
11. `plans/desktop/IPC.md` still says `ok`, has no `force`, `source` or null session id, and does not
    say which dates are snapshot dates. `types.ts` is what the fixtures prove.

## Verification

- `pnpm check`: tsc, eslint, 52 Playwright tests (webkit), `vite build` - all clean.
- `shots/` (gitignored, regenerated by the tests) holds 148 screenshots: 37 states x light/dark x
  1200x800/900x600, all with the generated demo data except the hand-written states above.
- Looked at after integration: every state at 1200x800 light and dark and at 900x600, the design's
  eleven beside `plans/desktop/design/shots`.
- Fixed from that review, each a rule and not a special case: the workspace path overflowing the
  sidebar at 900; a fix's sentence pushing its row out of the notice (Advisor refusal, empty state);
  the run summary's notice clipped by a no-wrap rule; a long ad set name pushing the Funnel's verdict
  column off screen; a failure sentence widening the steps table; an answer of several markdown blocks
  getting browser-default margins (and its caret a line of its own) because the renderer wrapped them
  in a div; a new session never being marked in the list (its id stayed null); the report picker
  collapsing to an empty box with no days; lower-case sentence starts; and two money styles on Brief.
- Fixed in the first build's review: the Brief feasibility grid, a double "Turn stopped" marker
  (StrictMode ran a state updater twice), the notice inside the empty state, spinners left running on
  dead runs and cut-short tool calls, and the report-none shot showing a report.
- Known rendering differences from the design shots, not defects: WebKit draws New York slightly
  heavier than the Chromium the mock was shot in, and rows sit about 1px differently.
- Not verified here: the real WKWebView inside Tauri, native window drag, and real IPC timing.
