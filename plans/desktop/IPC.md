# Moneyball for Mac - the IPC contract

The one seam between `crates/moneyball-desktop/src` (Rust, helper `shell`)
and `crates/moneyball-desktop/ui` (TypeScript, helper `ui`). Both build to
this file. If it is wrong or short, say so to the lead; do not drift.

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
- The webview has no network. It talks only through `ui/src/ipc/`.

## Shared types

```ts
type Tint = "plain" | "good" | "warn" | "bad";            // core words::Tint

/** A core `Command`, translated for an app without slash commands. */
type Action =
  | { kind: "refresh"; label: string; does: string }      // a button that starts a refresh
  | { kind: "report"; label: string; does: string }       // a button that generates the report
  | { kind: "terminal"; command: string; does: string };  // "In Terminal" + copy; never run by the app

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
  status: "ok" | "failed" | "skipped" | "waiting" | "running";  // waiting/running are UI-only states
  word: string;                                    // status_word
  detail: string;                                  // counts_detail or the failure message
  time: string | null;                             // duration(ms)
  reused: boolean;
  fix: Action | null;
};

type RunView = {
  date: string;            // the data day, "2026-10-08"
  day: string;             // "Thu 08 Oct"
  trigger: "manual" | "schedule";
  outcome: string;         // core Outcome, snake_case
  gaps: string;            // refresh::gaps, "" for a clean run
  steps: StepView[];       // always the four, in order
};
```

## Commands

| Command | Args | Returns | Notes |
|---|---|---|---|
| `workspace_current` | - | `Workspace \| null` | The remembered workspace, opened. `null` on a first run. `AppError` when the remembered folder is gone or is no workspace. |
| `workspace_pick` | - | `Workspace \| null` | Native folder dialog. `null` when cancelled. Opens, remembers, drops the old session. `AppError` when the folder has no `.moneyball/config.json` (v1 has no setup; `fix` is the terminal command). |
| `today` | - | `Today` | Recomputed on every call (reads the clock). Call on mount, after a refresh, and on window focus. |
| `brief` | - | `BriefView` | |
| `funnel` | `{ product, days, by }` | `FunnelView` | `days` 7/14/28; `by` "campaign"/"adset"/"ad". |
| `refresh_start` | `{ force?: boolean, onEvent }` | `void` | `AppError` kind `busy` if one is running. |
| `refresh_cancel` | - | `void` | Takes effect after the step in flight. |
| `refresh_state` | - | `RefreshState` | For mount/reload. |
| `report_days` | - | `string[]` | Snapshot dates, newest first. |
| `report_for` | `{ date?: string }` | `ReportView \| null` | The existing report for that day (default: the data day). `null` when none is written. |
| `report_generate` | `{ date?: string, days: number }` | `ReportView` | No network. `AppError` kind `busy` while a refresh runs (it writes the same file). |
| `report_open_in_browser` | `{ path }` | `void` | Refuses any path outside the workspace's `.moneyball/reports/`. |
| `sessions_list` | - | `SessionSummary[]` (core) | Most recently modified first. |
| `session_current` | - | `SessionSnapshot` | Creates a session lazily on first call. For mount/reload. |
| `session_open` | `{ id }` | `SessionSnapshot` | `AppError` kind `busy` while a turn runs. |
| `session_new` | - | `SessionSnapshot` | Same guard. |
| `turn_start` | `{ text, onEvent }` | `void` | `AppError` kinds: `busy`, `no_model`, `unknown_provider` (each with a sentence in `reason`), `not_a_command` when `text` starts with `/` (never sent to the model). |
| `turn_cancel` | - | `void` | The stream then ends with `turn_aborted`. |
| `copy_text` | `{ text }` | `void` | Clipboard, for the "In Terminal" copy button. |

```ts
type Today = {
  health: Health;            // core health::Health
  health_label: string;      // Health::label(), the footer's word ("Fresh", "CRM missing")
  footer: [string, string];  // the footer's two lines: data day, CRM day
  headline: { tint: Tint; text: string; rest: string } | null;   // core Landing.headline
  details: string[];
  fixes: Action[];
  next: Action | null;
  day: {                     // core DayBlock, with paths and commands translated
    day: string;
    refreshed: string | null;
    numbers: { label: string; value: string }[] | null;   // null = no report for the day
    numbers_note: string | null;                           // "no CRM in this report"
    versus: { day: string; changes: { label: string; text: string; tint: Tint }[] } | null;
    report_missing: { note: string; next: Action } | null;
  } | null;
  last_run: RunView | null;  // the "Data health" table
};

type BriefView = {
  brief: ProductRowsAndFeasibility | null;   // core wire; CRM fields null without CRM
  crm_note: string | null;
  fault: AppError | null;                    // a snapshot exists but would not load
  staleness: string | null;                  // brief::staleness_warning
  cpl_order: string | null;                  // brief::cpl_order
  feasibility_line: string | null;           // the one sentence under the table, worded in Rust
};

type FunnelView = {
  product: string;                           // the resolved name
  has_crm: boolean;
  rows: (FunnelRow & { verdict: string })[]; // core wire + funnel::verdict
  cpl_ranking: string | null;
  notes: string[];                           // the kill-rule and immature lines, worded in Rust
};

type RefreshState = { running: boolean; day: string; steps: StepView[]; started_ms_ago: number | null };

type RefreshEvent =
  | { type: "begin"; step: StepView }        // status "running"
  | { type: "done"; step: StepView }
  | { type: "finished"; run: RunView; time: string;
      verdict: { tint: Tint; headline: string; rest: string; next: Action | null } | null;
      fixes: Action[]; rerun: Action | null;
      numbers: { label: string; value: string }[] | null; numbers_note: string | null;
      report_path: string | null }
  | { type: "could_not_start"; error: AppError }
  | { type: "died"; message: string };       // channel closed with no final event

type ReportView = { date: string; day: string; days: number; path: string; html: string; summary: string };

type SessionSnapshot = {
  id: string | null;
  items: ViewItem[];        // core agent::ViewItem
  turn_active: boolean;     // true after a reload mid-turn: show "working", poll session_current
};

/** Core agent::Ev as is, plus one desktop variant. Deltas are coalesced (~30ms). */
type TurnEvent =
  | Ev
  | { type: "died"; message: string }                 // worker gone with no final event
  | { type: "save_failed"; message: string };         // shown once; the chat goes on
```

On `failed`, the desktop crate appends its own hint to `error` when
`looks_like_context_overflow` is true ("Start a new session to continue.").

## Fixtures

The shell writes one JSON example of every return type and every event
variant to `crates/moneyball-desktop/ui/src/ipc/fixtures/` from a Rust
test (pinned like docs/schemas; regen with `MB_UPDATE_FIXTURES=1`), built
from `fixtures/snap/` through the real command bodies. The UI imports
them with `satisfies` against `types.ts`, and its Playwright IPC mock
serves them. That is the drift alarm: a Rust shape change fails `tsc`.
Until the shell's fixtures exist the UI helper writes its own to the same
paths; the lead replaces them at integration.

## Not in v1

No subscribe-after-reload for a turn in flight (the snapshot says
`turn_active`; the UI polls). No in-step refresh progress: core reports a
step beginning and ending only.
