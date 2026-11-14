# Moneyball for Mac - the IPC contract

The one seam between `crates/moneyball-desktop/src` (Rust, helper `shell`)
and `crates/moneyball-desktop/ui` (TypeScript, helper `ui`). Both build to
this file. It describes exactly what is sent: `ui/src/ipc/types.ts` and
the pinned fixtures are the same contract in code, and the three change
together. If it is wrong or short, fix it in the same commit; do not drift.

Rules
- Command names are snake_case; arguments are camelCase in JS (Tauri's
  default mapping). Every command returns at once; long work streams over
  a `Channel` passed as the `onEvent` argument.
- Every failure is `AppError`. No command panics, none returns a secret.
- Types named "core" below serialize exactly as `moneyball-core` does;
  read the API notes (`api-core-data.md`, `api-core-words.md`,
  `api-core-agent.md`) and the wire structs they name. Everything else is
  defined here and built in the desktop crate.
- All sentences come from core or the desktop crate. The UI lays out and
  colours; it never composes product wording, never computes a figure,
  and shows `null` as an empty cell or the given note - never as 0.
- Nothing shown, and nothing the advisor is told, names a slash command
  (a Rust test holds this over every pinned fixture and prompt).
- The webview holds nothing it cannot re-read: every screen's state comes
  back from a command after a reload.
- The webview has no network. It talks only through `ui/src/ipc/`.

## Shared types

```ts
type Tint = "plain" | "good" | "warn" | "bad";            // core words::Tint

/** A core `Command`, translated for an app without slash commands. */
type Action =
  | { kind: "refresh"; label: string; does: string; force: boolean }  // a button that starts a refresh; force pulls Meta again
  | { kind: "report"; label: string; does: string }       // a button that generates the daily report for the current data day
  | { kind: "terminal"; command: string; does: string };  // "In Terminal" + copy; never run by the app

/** `kind` is an open string (busy, no_model, snapshot, ...): the UI shows `reason` and `fix` and never branches on it. */
type AppError = { kind: string; reason: string; fix: Action | null };

type Workspace = {
  name: string;          // folder name
  path: string;          // absolute
  display_path: string;  // "~/DEV/moneyball-test"
  products: string[];    // configured product names, in config order
};

/** One refresh step, worded by core (refresh/show.rs). */
type StepView = {
  name: "meta" | "assets" | "crm" | "report";
  label: string;                                   // step_label
  // core's StepStatus names, plus waiting and running for a live run
  status: "done" | "partial" | "failed" | "skipped" | "waiting" | "running";
  word: string;                                    // status_word
  detail: string;                                  // counts_detail, or why it failed, fell short or was skipped
  time: string | null;                             // duration(ms)
  note: string | null;                             // a line under the detail: "from today's earlier pull"
  fix: Action | null;
};

type RunView = {
  date: string;            // the data day, "2026-10-08"
  day: string;             // "Thu 08 Oct"
  trigger: "manual" | "schedule";
  outcome: string;         // core Outcome, snake_case
  gaps: string;            // refresh::gaps ("finished without CRM", "stopped at step 1"), "" for a clean run
  steps: StepView[];       // always the four, in order
};
```

## Commands

| Command | Args | Returns | Notes |
|---|---|---|---|
| `workspace_current` | - | `Workspace \| null` | The open workspace, its config re-read from disk; before one is open, the remembered workspace, opened. `null` on a first run. `AppError` when the remembered folder is gone or is no workspace. Call whenever `today` is called. |
| `workspace_pick` | - | `Workspace \| null` | Native folder dialog. `null` when cancelled. Opens, remembers, drops the old session. `AppError` when the folder has no `.moneyball/config.json` (v1 has no setup; `fix` is the terminal command), and kind `busy` while a refresh runs. A workspace that opened but could not be remembered is still returned; only the next launch forgets it. |
| `today` | - | `Today` | Recomputed on every call (reads the clock). Call on mount, after a refresh or a generated report, and on window focus. |
| `brief` | - | `BriefView` | With no snapshot yet, `fault` is the no-data `AppError` (kind `snapshot`, a `refresh` fix). |
| `funnel` | `{ product, days, by }` | `FunnelView` | `days` 7/14/28; `by` "campaign"/"adset"/"ad". With no snapshot yet: the no-data `AppError`. |
| `refresh_start` | `{ force?: boolean, onEvent }` | `void` | `AppError` kind `busy` if one is running. `force` pulls Meta again even when today's snapshot has it. |
| `refresh_cancel` | - | `void` | Takes effect after the step in flight. |
| `refresh_state` | - | `RefreshState` | For mount, reload and window focus. The run in flight while one runs; otherwise the open workspace's last recorded run, whoever made it. |
| `report_days` | - | `ReportDay[]` | The days with a snapshot, newest first. With none: the no-data `AppError` (never an empty list). |
| `report_for` | `{ date?: string }` | `ReportView \| null` | The daily report already written for that snapshot date (default: the newest). `null` when none is written. |
| `report_generate` | `{ date?: string, days: number }` | `ReportView` | No network. `date` is a snapshot date (default: the newest, the current data day); `days` is one of that day's `windows` and is held to 1..28. `AppError` kind `busy` while a refresh runs (it writes the same file). |
| `report_open_in_browser` | `{ path }` | `void` | Refuses any path outside the workspace's `.moneyball/reports/`. |
| `sessions_list` | - | `SessionSummary[]` (core) | The sessions this app wrote (the session header names its front-end; the terminal app's are left out), most recently modified first. |
| `session_current` | - | `SessionSnapshot` | The session on screen; creates nothing. For mount/reload. |
| `session_open` | `{ id }` | `SessionSnapshot` | Only a session this app wrote. `AppError` kind `busy` while a turn runs, `session` when it cannot be opened. |
| `session_new` | - | `SessionSnapshot` | Same `busy` guard. Creates no file: the first `turn_start` does. |
| `turn_start` | `{ text, onEvent }` | `void` | `AppError` kinds: `busy`, `no_model`, `unknown_provider` (each with a sentence in `reason`), `not_a_command` when `text` starts with `/` (never sent to the model), `empty` for a blank message. The model check reads the workspace config from disk, so a fix made in the terminal holds at once. |
| `turn_cancel` | - | `void` | The stream then ends with `turn_aborted`. |
| `copy_text` | `{ text }` | `void` | Clipboard, for the "In Terminal" copy button. `AppError` kind `io` when the copy failed. |

```ts
type Today = {
  health: Health;            // core health::Health
  health_label: string;      // Health::label(), the footer's word ("Fresh", "CRM missing")
  footer: [string, string];  // the footer's two lines: data day, CRM day
  headline: { tint: Tint; text: string; rest: string } | null;   // core Landing.headline
  details: string[];
  fixes: Action[];
  next: Action | null;
  day: {                     // the data day on screen, whatever the health (a stale day still has its report); null with no data
    day: string;                                           // "Thu 08 Oct"
    refreshed: string | null;                              // core landing::refreshed
    report: ReportView | null;                             // the day's daily report, as report_for returns it; null = none written
    report_missing: { note: string; next: Action } | null; // with no report: the sentence, and a `report` Action
  } | null;
  last_run: RunView | null;  // the run behind the data; Today shows its `gaps`, Refresh its steps
};

/** A day number; `source` (absent for a label the shell does not know) colours its dot. */
type DayNumber = { label: string; value: string; source?: "meta" | "crm" };

type BriefView = {
  brief: ProductRowsAndFeasibility | null;   // core wire; CRM fields null without CRM
  crm_head: string | null;                   // "CRM columns hidden.", said in front of crm_note when the table has none
  crm_note: string | null;
  fault: AppError | null;                    // a snapshot exists but would not load
  staleness: string | null;                  // worded here from brief::stale_days: when it was pulled, and to refresh
  cpl_order: string | null;                  // brief::cpl_order, the whole line: "cheapest Meta lead first: ..."
  caption: string | null;                    // what the table's columns count
  feasibility_line: string | null;           // the one sentence under the table, worded in Rust
};

type FunnelView = {
  product: string;                           // the resolved name
  has_crm: boolean;
  rows: (FunnelRow & { verdict: string })[]; // core wire + funnel::verdict
  cpl_ranking: string | null;
  notes: string[];                           // the kill rule and what immature means, or why kill is withheld
                                             // without CRM data; the UI adds no sentence of its own
};

/** A finished run in full, built from its run record alone. */
type Finished = {
  run: RunView; time: string;                // time: "1m 21s"
  verdict: { tint: Tint; headline: string; rest: string; next: Action | null } | null;  // null for a clean run
  fixes: Action[]; rerun: Action | null;
  numbers: DayNumber[] | null; numbers_note: string | null;   // null when the run wrote no report
  report_path: string | null;
};

type RefreshState = {
  running: boolean;
  day: string;                    // the data day the run fills, "Thu 08 Oct"; "" with nothing to show
  steps: StepView[];              // the four, or [] before any run
  started_ms_ago: number | null;  // only while running
  // The last run as its `finished` event gave it, built from the record by the same code. null
  // while a run is in flight, before any run, and for a record still marked running (a run in
  // flight elsewhere, or one that died: its steps are shown, with no verdict).
  finished: Finished | null;
};

type RefreshEvent =
  | { type: "begin"; step: StepView }        // status "running"
  | { type: "done"; step: StepView }
  | ({ type: "finished" } & Finished)
  | { type: "could_not_start"; error: AppError }   // final; re-read refresh_state for the last recorded run
  | { type: "died"; message: string };       // channel closed with no final event

/** A day with a snapshot. `date` is the snapshot date the report commands take; `day` words the data day, one day earlier. */
type ReportDay = {
  date: string;                              // "2026-10-09"
  day: string;                               // "Thu 08 Oct"
  // The windows a report ending that day can cover, worded in Rust: "1 day" (1), "7 days" (7) and
  // "Month to date" (the data day's day of the month) while that fits the 28 days a snapshot
  // holds; past the 28th the third is "Last 28 days" (28). The UI renders them and sends `days`.
  windows: { label: string; days: number }[];
};

/** `date` is the snapshot date; `day` the last day covered, in words; `days` the complete days covered. */
type ReportView = { date: string; day: string; days: number; path: string; html: string; summary: string };

type SessionSnapshot = {
  id: string | null;        // null until the session's first turn creates its file
  items: ViewItem[];        // core agent::ViewItem
  turn_active: boolean;     // true after a reload mid-turn: show "working", poll session_current
  failed: string | null;    // how the last turn failed or died, until the next one starts (the file has no item for it)
};

/** Core agent::Ev as is, plus one desktop variant. Deltas are coalesced (~30ms). */
type TurnEvent =
  | Ev
  | { type: "died"; message: string }                 // worker gone with no final event
  | { type: "save_failed"; message: string };         // shown once; the chat goes on
```

On `failed`, the desktop crate appends its own hint to `error` when
`looks_like_context_overflow` is true ("Start a new session to continue.").
A `failed` or `died` sentence is also kept on `SessionSnapshot.failed`.

## What the UI re-reads, and when

| When | Commands |
|---|---|
| Mount (and a workspace switch, which remounts every screen) | `workspace_current`, `today`, `refresh_state`, `session_current`, `sessions_list`, each screen's own |
| Window focus | `today`, `workspace_current`, `refresh_state` |
| A refresh settles, or a report is generated | `today`, `workspace_current`, and every screen that reads the snapshot or the reports |

A `report` Action (Today, a refresh verdict) generates the daily report
for the current data day: `report_generate({ days: 1 })` with no date,
whatever the Report screen had selected, and the screen's selections go
back to that day and "1 day".

## Fixtures

The shell writes one JSON example of every return type and every event
variant to `crates/moneyball-desktop/ui/src/ipc/fixtures/` from a Rust
test (pinned like docs/schemas; regen with `MB_UPDATE_FIXTURES=1`), built
from `crates/moneyball-desktop/fixtures/` through the real command bodies.
The UI checks each against `types.ts` (`ui/src/ipc/fixtures.ts`), and its
Playwright IPC mock serves them. That is the drift alarm: a Rust shape
change fails `tsc`.

## Not in v1

No subscribe-after-reload for a turn in flight (the snapshot says
`turn_active`; the UI polls). No in-step refresh progress: core reports a
step beginning and ending only.
