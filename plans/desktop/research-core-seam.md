# Desktop front-end: the core seam as it stands

Read-only research, 2026-11-12, at commit 83f5a85. Scope: a new crate
`crates/moneyball-desktop` (Tauri 2, macOS) depending on `moneyball-core`
only, beside `moneyball-tui`. Paths are relative to the repo root; `core/`
= `crates/moneyball-core/src/`, `tui/` = `crates/moneyball-tui/src/`,
`bin/` = `crates/moneyball/src/`.

Headline findings:

- Core already holds all the math and all the I/O. What it lacks is the
  glue: every worker thread, every channel, the tool executor, the system
  prompts, and almost all user-facing sentences live in the TUI crate.
- There is no `Op` type anywhere in the code. ARCHITECTURE.md:168 names it;
  only `Ev` exists (core/agent.rs:46). The three "ops" are direct calls and
  an AtomicBool in the TUI (section 3).
- `Ev`, `Health`, the brief rows, the funnel rows and `ReportOutput` do not
  derive `Serialize`, so none can cross the Tauri IPC boundary as is
  (section 4).
- None of the four arch_contract tests scan a fourth crate: each has a
  hardcoded crate list (section 5).
- Launched from Finder, workspace resolution lands on `/moneyball-data`
  and reports "no workspace" (section 6).

Docs that the new crate contradicts as written (not test-enforced):
AGENTS.md:8-9 "TUI is the management surface; no web UI";
ARCHITECTURE.md:13-23 "Crate topology (locked)" with the dependency rule
`moneyball -> moneyball-tui -> moneyball-core`.

---

## 1. What core provides per v1 surface

Common entry for every surface:

| Need | Core item | Ref |
|---|---|---|
| Config | `AppConfig { data_root: PathBuf, date: Option<String>, workspace: Option<WorkspaceConfig>, agent: bool }` | core/config.rs:197-203 |
| Resolve (never errors) | `AppConfig::resolve_optional(data_root: Option<&str>, date: Option<&str>) -> Self` | core/config.rs:224 |
| Resolve (strict) | `AppConfig::resolve(data_root: Option<&str>, date: Option<&str>) -> Result<Self>` | core/config.rs:207 |
| Snapshot dir | `AppConfig::snap_for(&self, date: Option<&str>) -> Result<PathBuf>` (None = latest dated dir) | core/config.rs:261 |
| Load snapshot | `snapshot::load(snap_path: &Path) -> Result<Snapshot>` | core/snapshot.rs:146 |
| Snapshot dates | `snapshot::list_dates(snap_root: &Path) -> Result<Vec<String>>` | core/snapshot.rs:235 |
| Dirs | `mb_dir()`, `snap_dir()`, `state_dir()`, `history_dir()`, `runs_dir()`, `sessions_root() -> Option<&Path>` | core/config.rs:253-321 |
| Errors | `Error` with `kind() -> &'static str`, `reason() -> String`, `fix() -> Option<String>` | core/error.rs:8, 57, 73, 85 |

### 1a. Data-health landing

| Core item | Signature | Ref |
|---|---|---|
| `Health` | struct: `today, snap_date, crm_snap_date, crm_connected, run: Option<RunRecord>, refreshed_at: Option<DateTime<Local>>, failed_today: Option<RunRecord>, newest_snap_date, state: State` | core/health.rs:35-51 |
| `State` | enum `Failed, NoData, Pinned, Stale, CrmMissing, Partial{have,of}, Fresh` | core/health.rs:15-33 |
| `Health::of` | `(cfg: &AppConfig, snap: Option<&Snapshot>) -> Health` (file reads only) | core/health.rs:54 |
| helpers | `age_days() -> Option<i64>`, `data_day() -> Option<NaiveDate>`, `crm_day()`, `label() -> String` | core/health.rs:98-121 |
| `STALE_DAYS` | `i64 = 2` | core/health.rs:12 |
| CRM state | `CrmHealth::of(cfg, snap: &Snapshot) -> Self`, `note() -> Option<String>` | core/crm/status.rs:18, 34, 52 |
| Day's report | `report::daily(cfg, snap_date: &str) -> Option<(CreativeReport, PathBuf)>` | core/report/exec.rs:184 |
| Run records | `runs::read(cfg, date: &str) -> Option<RunRecord>`, `runs::latest(cfg)` | core/runs.rs:26, 32 |
| "when" wording | `status::when(t: DateTime<Local>, today: NaiveDate) -> String` | core/status.rs:210 |
| Step wording | `refresh::step_label`, `refresh::duration(ms) -> String` | core/refresh/show.rs:22, 67 |
| Schedule flag | `schedule::installed(cfg) -> Option<(u32, u32)>` | core/schedule.rs:109 |

Core stops at `Health` + `label()`. Every sentence on the landing is in
the TUI (section 2, item L).

### 1b. Brief

| Core item | Signature | Ref |
|---|---|---|
| `brief::compute` | `(snap: &Snapshot, cfg: &AppConfig, history: &[HistoryRow]) -> ProductRowsAndFeasibility` | core/brief/mod.rs:57 |
| `brief::load_history` | `(path: &Path) -> Vec<HistoryRow>` (path = `history_dir()/scoreboard.csv`) | core/brief/history.rs:9 |
| `ProductRowsAndFeasibility` | `rows: Vec<ProductRow>, feasibility: Feasibility, snapshot_date: String, has_crm: bool` | core/brief/mod.rs:326-335 |
| `ProductRow` | `product, spend_per_day, m7d, cpl, l7d, q7d, q_per_day, rs_per_q, l_to_q, goal, gap, trend` | core/brief/mod.rs:28-43 |
| `Feasibility` | `tot_q_per_day, tot_spend_per_day, tot_goal_per_day, cur_rpq, required_at_cur, required_at_best, best_rpq, open_debt` | core/brief/mod.rs:46-55 |
| `brief::staleness_warning` | `(snapshot_date: &str) -> Option<String>` | core/brief/mod.rs:340 |
| `brief::cpl_order` | `(rows: &[ProductRow]) -> Option<String>` | core/brief/print.rs:116 |
| `brief::brief_json` | `(r: &ProductRowsAndFeasibility, crm_note: Option<String>) -> serde_json::Value` (CRM fields null when no CRM) | core/brief/print.rs:135 |
| `brief::format_brief_table` | `(rows: &[ProductRow], has_crm: bool) -> String` | core/brief/print.rs:67 |
| `brief::run` | headless, prints to stdout | core/brief/mod.rs:351 |

### 1c. Funnel

| Core item | Signature | Ref |
|---|---|---|
| `funnel::By` | enum `Campaign/Adset/Ad`; `By::parse(&str) -> Result<Self>`, `name() -> &'static str` | core/funnel/mod.rs:26, 33, 44 |
| `funnel::compute` | `(snap, cfg, product: &str, days: u32, by: By) -> Vec<FunnelRow>` (sorted by spend desc) | core/funnel/mod.rs:97 |
| `funnel::compute_between` | `(snap, cfg, product, (d0, d1), by)` | core/funnel/mod.rs:113 |
| `FunnelRow` | `id, name, spend, m, l, q, v, b, cpl, rs_per_q, l_to_q, kill_mult, kill: Option<bool>, sufficient, immature, learning` | core/funnel/mod.rs:61-81 |
| `funnel::verdict` | `(r: &FunnelRow) -> &'static str` ("no CRM", "wait: ...", "kill", "keep") | core/funnel/print.rs:65 |
| `funnel::table` / `view` | `(rows) -> String` (model table / human table, ASCII grids) | core/funnel/print.rs:96, 101 |
| `funnel::cpl_ranking` / `cpl_ranking_for_model` | `(rows) -> Option<String>` | core/funnel/mod.rs:269, 274 |
| `KILL_TABLE`, `LAG_HOURS` | consts | core/funnel/mod.rs:19, 21 |
| `product::resolve_configured` | `(cfg, name: &str) -> Result<String, String>` (Err lists candidates) | core/product.rs:9 |

### 1d. Refresh with live step table

| Core item | Signature | Ref |
|---|---|---|
| `refresh::run` | `(cfg: &AppConfig, meta_token: Option<&str>, opts: &Options, on_step: &mut dyn FnMut(Progress)) -> Result<RunRecord>`; blocking; Err only when it cannot start (no workspace, lock held) | core/refresh/mod.rs:48 |
| `refresh::Options` | `trigger: Trigger, force: bool, days: u32, cancel: Option<Arc<AtomicBool>>` | core/refresh/mod.rs:34-43 |
| `refresh::Progress<'a>` | `Begin(&'static str)`, `Done(&'a RunStep)` (borrowed) | core/refresh/mod.rs:26-29 |
| `refresh::STEPS` | `["meta", "assets", "crm", "report"]` | core/refresh/show.rs:20 |
| `refresh::CANCELLED` | `"cancelled"` (a skipped step's error_kind) | core/refresh/mod.rs:32 |
| Step wording | `step_label`, `status_word(&RunStep)`, `counts_detail(&RunStep)`, `duration(ms)`, `step_line(n, &RunStep)`, `REUSED_NOTE`, `data_day_label()` | core/refresh/show.rs:9-95 |
| `RunRecord` | `schema, workspace_id, date, trigger, started_at, finished_at, outcome, steps, artifacts: BTreeMap<String,String>` (keys seen: "snapshot", "report", "report_json") | core/schema/run.rs:13-31; core/refresh/mod.rs:158, 267-270 |
| `RunStep` | `name, status, ms, counts, error_kind, message, fix, reused` | core/schema/run.rs:52-74 |
| `Trigger`, `Outcome`, `StepStatus` | enums | core/schema/run.rs:35, 44, 78 |
| Meta token | `secrets::load_meta_token() -> Option<String>` (caller resolves at the edge) | core/secrets.rs:133 |
| Lock | `runs::lock(cfg) -> Result<RunLock>`; never waits; taken inside `refresh::run` | core/runs.rs:47; core/refresh/mod.rs:54 |
| History of runs | `status::days(cfg, n) -> Vec<Day>` | core/status.rs:64 |

Cancel granularity: checked only between steps (core/refresh/mod.rs:200,
245, 302-306); a Meta pull in flight is not cut.

### 1e. Creative report viewer

| Core item | Signature | Ref |
|---|---|---|
| `report::generate` | `(cfg, date: Option<&str>, window_days: u32) -> Result<ReportOutput>`; no network; writes `reports/<date>/creative-report[-range].json` and `<Brand>-Daily-<date>.html` | core/report/mod.rs:227, 251-318 |
| `ReportOutput` | `report: CreativeReport, json_path: PathBuf, html_path: PathBuf` | core/report/mod.rs:218-222 |
| `report::daily` | `(cfg, snap_date) -> Option<(CreativeReport, PathBuf)>` (existing report for a day) | core/report/exec.rs:184 |
| `report::text_summary` | `(r: &CreativeReport) -> String` | core/report/summary.rs:11 |
| `report::build` | pure aggregate from a `Snapshot` | core/report/mod.rs:40 |
| `html::render` | `(r, prior: Option<&CreativeReport>, brand: &str, history_dir: &Path) -> String` | core/report/html.rs:32 |
| `CreativeReport` + parts | `ProductSection, CreativeCard, Kpis, FunnelCounts, Verdict, ...` | core/schema/report.rs:16-328 |

The HTML is self-contained: images are downscaled and inlined as
`data:` URIs (core/report/html.rs:173-175, core/report/img.rs:2), so the
file loads in a webview with no asset protocol access to `history/assets/`.

### 1f. Advisor chat with sessions

| Core item | Signature | Ref |
|---|---|---|
| `agent::Item` | enum `User{text} / Assistant{text} / ToolCall{call_id,name,args} / ToolOutput{call_id,output,is_error}` | core/agent.rs:20-40 |
| `agent::Ev` | see section 3 | core/agent.rs:46-80 |
| `agent::run_turn` | `(provider_id: &str, provider: &ModelProviderInfo, model: &str, system: &str, history: Vec<Item>, tools: &[Tool], exec: &dyn ToolExec, cancel: &Arc<AtomicBool>, tx: &Sender<Ev>)`; blocking, runs on the caller's thread | core/agent.rs:165-175 |
| `agent::ToolExec` | `trait ToolExec: Send { fn run(&self, name: &str, args: &Value) -> Result<String, String>; }` | core/agent.rs:85-87 |
| `agent::heal_history`, `truncate_output`, `TURN_ABORTED_MARKER` | pub | core/agent.rs:122, 93, 117 |
| `agent::Usage` | `input: u64, output: u64` | core/agent.rs:356-359 |
| Tool definitions | `tools::brief_tool()`, `funnel_tool()`, `diagnose_tool()`, `recommend_tool()` -> `Tool` | core/tools.rs:51, 67, 91, 111 |
| Provider | `WorkspaceConfig.model_provider`, `.model`, `.model_providers: HashMap<String, ModelProviderInfo>` | core/config.rs:91, 94, 99 |
| Key | `ModelProviderInfo::api_key(&self, provider_id) -> Option<String>` (auth.json, then `env_key` env var) | core/provider.rs:74-93 |
| Ledger write | `ledger::submit(cfg, &snap, rec: Rec, now) -> Result<id, Vec<String>>` | core/ledger/submit.rs:12 |
| Diagnose | `diagnose::compute(snap, cfg, product) -> Diagnosis`, `diagnose::text(&Diagnosis) -> String` | core/diagnose/mod.rs:118; core/diagnose/print.rs:14 |
| `SessionLog::create` | `(data_root: PathBuf, root: Option<&Path>) -> anyhow::Result<Self>` | core/session/mod.rs:57 |
| `SessionLog::open` | `(id: &str, root: Option<&Path>) -> anyhow::Result<(Self, Vec<Item>)>` | core/session/mod.rs:82 |
| `SessionLog::append` | `(&self, item: &Item) -> anyhow::Result<()>` | core/session/mod.rs:92 |
| `SessionMeta` | `id: String, started_at: DateTime<Utc>, data_root: PathBuf` | core/session/mod.rs:29-33 |
| `session::list` | `(root: Option<&Path>) -> Result<Vec<SessionMeta>>` (newest first; reads header lines only) | core/session/mod.rs:172 |
| `session::latest_id`, `read_raw`, `fmt_meta_line`, `sessions_dir` | pub | core/session/mod.rs:187, 164, 192, 129 |
| Markers | `debug::is_bug_report_marker(text) -> bool` | core/debug/reports.rs:60 |

Gaps for a session list UI: `SessionMeta` has no title, preview or item
count (core/session/mod.rs:29-33); `list` returns id + start time + root
only. A preview needs `SessionLog::open` per session.

---

## 2. Logic that lives only in the TUI crate

Sizes are lines of the named range. "Move" = into core; "Duplicate" =
re-implement in the desktop crate or its JS.

| # | What | Where | Size | ratatui-coupled? | Recommendation |
|---|---|---|---|---|---|
| A | Agent turn start: provider/model lookup, one-turn guard, record the turn, reset cancel, spawn worker, build tool list, call `run_turn` | tui/agent_turn.rs:119-205 (`call_agent`) | 87 | Only the cell pushes (131-135, 141-147, 155-156, 162-165) | Move the non-UI half (122-150 lookup, 166-199 spawn) into core as one "start a turn" fn returning the `Receiver<Ev>`; each UI keeps its own "no LLM configured" presentation. The inner forwarder thread (177-184) only wraps `Ev` into the TUI's `StreamEvent` and is not needed elsewhere. |
| B | Tool executor `SnapshotTools: ToolExec` (brief, funnel, diagnose, recommend) | tui/agent_tools.rs:8-82 | 82 (whole file) | No. Holds only an `AppConfig` | Move. The comment at core/tools.rs:43-45 ("handlers live in moneyball-tui, they need App") is stale. Duplicating would fork what the model sees. |
| C | Model-facing tool text: `brief_text`, `brief_output`, `format_brief_as_lines`, `cpl` | tui/brief_flow.rs:72-186 | 115 | No | Move with B (B calls `brief_output` at agent_tools.rs:28). |
| C2 | `funnel_output` | tui/funnel_flow.rs:10-33 | 24 | No | Move with B (agent_tools.rs:37). |
| C3 | `diagnose_output` | tui/diagnose_flow.rs:20-30 | 11 | No | Move with B (agent_tools.rs:47). |
| D | System prompts: `RECORD_RULE`, `AGENT_SYSTEM_PROMPT`, `FUNNEL_SYSTEM_PROMPT`; `BRIEF_SYSTEM_PROMPT`; `DIAGNOSE_SYSTEM_PROMPT` | tui/agent_turn.rs:10-29; tui/brief_flow.rs:11-19; tui/diagnose_flow.rs:9-16 | 20 + 9 + 8 | No | Move. One advisor persona for both front-ends. |
| E | `app_state_block(app) -> String`: today, workspace, products + goals, snapshot date, CRM note, slash-command list, output-style rule | tui/agent_turn.rs:34-86 | 53 | No, but reads `App` (`cfg`, `snap_date`, `crm_note`) and `COMMANDS` | Move, with the two TUI-specific lines as inputs: the command list (77-78) and the "plain terminal prose ... tables, links and nested lists do not render" rule (79-84). The desktop must pass only commands it implements (CLAUDE.md hard rule) and its own rendering rule. |
| F | `typed_tool_turn`: User + ToolCall + ToolOutput items for a typed command | tui/agent_turn.rs:91-115 | 25 | No (uses `app.history.len()` for the call id) | Move. |
| G | `run_freeform` | tui/agent_turn.rs:210-217 | 8 | No | Duplicate (trivial once A, D, E are in core). |
| H | `App::record`: push to history + `SessionLog::append`, surface a save failure once | tui/app.rs:208-215 | 8 | No | Duplicate, or move with A if core owns history + log together. |
| I | `App::replay`: `Vec<Item>` -> visible cells; hides `<turn_aborted>` and bug-report markers; pairs a ToolOutput with the preceding ToolCall name | tui/app.rs:219-270 | 52 | Yes (cells) | Duplicate in the frontend from serialized `Item`s. The marker rules (228-234) are the part that must match; `TURN_ABORTED_MARKER` and `is_bug_report_marker` are already pub in core. |
| J | Event drain: `Ev` -> cells, tool timing, status line, turn end | tui/stream_drain.rs:11-158 | 148 | Yes | Duplicate (the desktop forwards each `Ev` to the webview). Shareable parts below. |
| J2 | `looks_like_context_overflow` + its "/clear" hint | tui/stream_drain.rs:160-174, 114-120 | 15 + 7 | No | Move the predicate; the hint names a TUI command. |
| J3 | Turn-complete status string (duration, context tokens, out tokens, provider) | tui/stream_drain.rs:80-96 | 17 | No | Duplicate (presentation). |
| J4 | Worker-died handling on `Disconnected` | tui/stream_drain.rs:141-155 | 15 | Partly | Duplicate the behavior: never a silent dead turn. |
| K | Session bootstrap: create or resume at start | tui/lib.rs:107-128 | 22 | Partly | Duplicate. |
| K2 | `--list`, `--resume <id>`, `-c` handling | bin/main.rs:174-212 | 39 | No (binary crate) | Duplicate (core primitives suffice). |
| K3 | `/clear`: fresh history + new session file, guarded while a turn runs | tui/commands.rs:231-275 | 45 | Partly | Duplicate. |
| L | Landing: the data-health sentences. `landing` 25-64, `story` 67-183, `next_command` 187-228, `day_block` 232-296, `versus` 300-332, `signed` 334-337, `how` 345-350, `missed` 354-366, `crm_behind` 369-383, `elapsed_ms` 385-391 | tui/landing.rs | 395 (+ 86 tests in landing_tests.rs) | Yes: every sentence is built into styled `Para` runs with `ratatui::style::Color` | Move the wording into core as unstyled structured output (headline, tone, detail lines, fixes, next command, day block); the TUI keeps only styling. Largest item. Duplicating forks ~250 lines of sentence logic that core's `Health` exists to feed. |
| M | `day_numbers(&CreativeReport) -> String` ("spend Rs.. . Meta leads ..") | tui/widgets.rs:141-163 | 23 | No (plain String) | Move (used by the landing and the refresh end state). |
| M2 | `fix_does(fix: &str) -> &'static str` (what a fix command does) | tui/widgets.rs:166-178 | 13 | No | Move with L. |
| N | Refresh worker + channel: `Ev` enum 19-24, `RefreshJob` 28-34, `run_refresh` 38-92 (spawn 67-85), `cancel` 96-103 | tui/refresh_flow.rs | 85 | Only the cell push (54-57) and notes | Move an owned, serializable event enum (Begin / Done(RunStep) / Finished{run, ms} / CouldNotStart{err}) and the spawn body (58-85) into core. `refresh::Progress` borrows its `RunStep` (core/refresh/mod.rs:26-29) so it cannot cross a channel as is; that is why the TUI clones at refresh_flow.rs:75. |
| N2 | Refresh drain: events -> step rows, step counter, worker-died state | tui/refresh_flow.rs:106-147 | 42 | Yes | Duplicate. |
| N3 | Refresh finish: read `report_json` for the day's numbers, set report path, reload brief, optional brief follow-up | tui/refresh_flow.rs:149-189 | 41 | Partly | Duplicate; lines 156-162 (numbers from the run's report json) are plain data and could move with M. |
| O | Refresh cell state + drawing: `Row`, `End`, `RefreshCell` | tui/refresh_cell/mod.rs | 310 | Yes | Duplicate (the desktop draws its own table from `RunStep`s). |
| O2 | Finished-run wording: `gaps` 25-38, `verdict` 42-102, `cancelled` 12-14 | tui/refresh_cell/words.rs | 102 | `verdict` returns a styled `Para` (97-101); the sentences are plain | Move the sentences beside core/refresh/show.rs, which already owns step wording. |
| P | Report worker: arg parse 23-35, busy guards 10-20, spawn 46-62, done/failed presentation 65-97 | tui/creative_flow.rs | 97 | Partly | Duplicate (spawn is 15 lines over `report::generate`). Keep the guard at 15-20: a running refresh writes the same day's report. |
| Q | `App::load_brief`: snap_for -> load -> load_history -> compute -> CRM note -> `Health::of`, with the three failure branches | tui/app.rs:272-302 | 31 | No | Move as one loader; both front-ends need the same bundle and the same "no snapshot is not a fault" branch (294-300). |
| R | `/brief` flow incl. self-heal through refresh when a Meta token exists | tui/brief_flow.rs:24-70 | 47 | Partly | Duplicate. |
| S | `/funnel` flow: trailing-level arg parse 41-45, usage message, table cell, hand-off to the advisor | tui/funnel_flow.rs:35-108 | 74 | Partly | Duplicate (arg parsing is unneeded with form controls). |
| T | Command registry + dispatch: `COMMANDS` 9-54, `submit` 64-155, slash-typo guard 143-149 | tui/commands.rs | 292 (whole file) | Yes (`App` input state, cells) | Do not move. If the chat box accepts slash commands, duplicate a small dispatcher and keep the typo guard (a mistyped command must never reach the paid LLM path). |
| U | Open report in browser via `open` | tui/refresh_flow.rs:203-228 | 26 | No | Not needed (webview or Tauri opener). |
| V | `compact_args` | tui/app.rs:328-333 | 6 | No | Duplicate. |
| W | Interrupt handling (Esc) | tui/event.rs:211-241 | 31 | Yes | Duplicate: set the cancel flag; a report job cannot be cut. |
| X | Setup wizard (workspace path, Meta token, products, LLM provider + key) | tui/setup/ | 1,824 | Yes | Out of v1 scope, but every v1 surface is empty without a configured workspace, and only this wizard creates one (tui/setup/steps.rs:34-45, 114-148). |

Related strings in core that assume a slash-command UI (not TUI-only, but
the desktop inherits them): `RunStep.fix` values such as `/connect crm`
(core/refresh/mod.rs:206), `CrmHealth::note` "Fix: /connect crm (moneyball
connect crm in a shell)" (core/crm/status.rs:58), the tool error "tell the
user to run /refresh" (tui/agent_tools.rs:18-19), and the prompts in D.

---

## 3. The Op/Ev seam and the life of a turn

### Types

`Op` does not exist. `grep -rn "enum Op\|Op::" crates` returns nothing.
ARCHITECTURE.md:168 describes `Op` in (UserInput / Interrupt / Shutdown);
in code those are:

| Documented Op | Actual mechanism | Ref |
|---|---|---|
| UserInput | direct call `call_agent(app, system, turn)` which spawns a thread | tui/agent_turn.rs:119 |
| Interrupt | `app.cancel.store(true, SeqCst)` on a shared `Arc<AtomicBool>` | tui/event.rs:215-216; field at tui/app.rs:64 |
| Shutdown | `app.quit = true`; the receiver drops, the forwarder's `send` fails and it exits; the worker is not joined | tui/commands.rs:109; tui/agent_turn.rs:179-182 |

`Ev` (core/agent.rs:46-80), no derives at all:

```rust
pub enum Ev {
    AssistantDelta(String),
    AssistantDone { text: String },
    ToolBegin { call_id: String, name: String, args: Value },
    ToolEnd { call_id: String, output: String, ok: bool },
    ItemDone(Item),
    TurnComplete { ms: u64, provider: String, usage: Usage },
    TurnAborted,
    Failed { error: String },
}
```

Transport: `std::sync::mpsc::Sender<Ev>` passed to `run_turn`
(core/agent.rs:174). The TUI wraps it once more:
`StreamEvent::{Agent(Ev), ReportDone{out, ms}, ReportFailed{err, ms}}`
(tui/app.rs:82-94) on the single `app.stream` slot (tui/app.rs:76), so a
running `/report` and an agent turn exclude each other
(tui/creative_flow.rs:10-14, tui/commands.rs:70-73). Refresh has its own
channel (tui/refresh_flow.rs:28-34).

### Start

1. Guard: one turn at a time, `app.stream.is_some()` rejects
   (tui/agent_turn.rs:154-158; also tui/commands.rs:70-73).
2. Provider id, provider info and model are read from the workspace config;
   missing ones produce an assistant message, not an error
   (tui/agent_turn.rs:122-150).
3. The turn's items (the user item, plus a synthetic tool round for a typed
   command) are recorded: history + session file (tui/agent_turn.rs:159-161
   -> tui/app.rs:208-215).
4. Cancel flag reset to false (tui/agent_turn.rs:166).
5. System prompt = persona + `app_state_block` + `RECORD_RULE`
   (tui/agent_turn.rs:171, 212).
6. A thread is spawned with a clone of the full history, the config and
   the cancel flag (tui/agent_turn.rs:172-175); it builds `SnapshotTools`
   and the four tool definitions and calls `agent::run_turn`
   (tui/agent_turn.rs:185-196).
7. `turn_active = true`, `stream = Some(rx)` (tui/agent_turn.rs:200-202).

### Stream

- `run_turn` heals dangling tool calls once (core/agent.rs:215), then loops:
  request -> tool calls -> outputs -> repeat until a response has no tool
  calls, no iteration cap (core/agent.rs:217-349).
- Text arrives as `AssistantDelta` (core/agent.rs:218-220); a finished
  message is `ItemDone(Item::Assistant)` then `AssistantDone`
  (core/agent.rs:253-256).
- Each tool call: `ItemDone(ToolCall)`, `ToolBegin`, execute,
  `ItemDone(ToolOutput)`, `ToolEnd` (core/agent.rs:304-338). Output is
  capped at 2000 lines / 50 KB (core/agent.rs:90-113, 322).
- A max_tokens stop with tool calls fails every call with an error output
  and loops (core/agent.rs:257-295); with plain text it appends a "cut off"
  note (core/agent.rs:247-252).
- End: `TurnComplete { ms, provider, usage }` (core/agent.rs:296-302).
- UI side: the 100 ms tick calls `stream_drain::drain` (tui/event.rs:37),
  which applies each `Ev` (tui/stream_drain.rs:19-128).

### Cancel

- UI sets the flag (tui/event.rs:215-216).
- `llm::stream_turn` checks it between SSE events and returns Err after
  dropping the stream (core/llm.rs:150-163).
- `run_turn` on that Err with the flag set: emits
  `ItemDone(Item::User{TURN_ABORTED_MARKER})` then `TurnAborted`
  (core/agent.rs:223-231). Without the flag the same Err is `Failed`
  (core/agent.rs:232-236).
- Between tool calls: after emitting a `ToolCall` it breaks without running
  it (core/agent.rs:313-315), then emits the marker and `TurnAborted`
  (core/agent.rs:339-347). The unanswered call is healed at the start of
  the next turn, in memory only (core/agent.rs:122-151, 210-215).
- Partial assistant text of an aborted response is never persisted (no
  `Item::Assistant` is emitted on that path).
- The UI clears `turn_active` and `stream` on `TurnAborted`
  (tui/stream_drain.rs:101-109).

### Persist

- Format: append-only JSONL; line 1 `{"session": SessionMeta}`, then one
  `Item` per line (core/session/mod.rs:37-42, 71-74, 92-99).
- Location: `MONEYBALL_SESSIONS_DIR` > `<workspace>/.moneyball/sessions/` >
  `~/.moneyball/sessions/` (core/session/mod.rs:129-144); a workspace
  lookup falls back to the global dir for older ids
  (core/session/mod.rs:150-160).
- Written by the UI thread, not the worker: user items before the spawn
  (tui/agent_turn.rs:159-161), then every `Ev::ItemDone` as it arrives
  (tui/stream_drain.rs:65-70). Terminal events carry no items.
- A failed append is shown once and does not stop the turn
  (tui/app.rs:209-213).
- Resume: `SessionLog::open` returns the handle and `Vec<Item>`;
  unparseable lines are skipped (core/session/mod.rs:82-88, 113-121);
  `App::replay` rebuilds cells and sets `history` (tui/app.rs:219-270).
  Tool durations are not persisted (tui/app.rs:262-264).
- New conversation: a new file; the old one is never rewritten
  (tui/commands.rs:255-258).

---

## 4. Serde status of the types a desktop IPC layer needs

chrono's `serde` feature is on workspace-wide (Cargo.toml:40), so chrono
fields are not a blocker.

### Already `Serialize`

| Type | Derives | Ref |
|---|---|---|
| `agent::Item` | Serialize, Deserialize (tagged `type`, snake_case) | core/agent.rs:20-22 |
| `session::SessionMeta` | Serialize, Deserialize | core/session/mod.rs:28 |
| `schema::RunRecord`, `RunStep`, `Trigger`, `Outcome`, `StepStatus` | Serialize, Deserialize, JsonSchema | core/schema/run.rs:12, 33, 42, 51, 76 |
| `schema::CreativeReport` and every nested type | Serialize, Deserialize, JsonSchema | core/schema/report.rs:15-328 |
| `schema::LedgerEvent`, `Rec`, `RecState`, `Evidence`, `Prediction`, `BudgetMove` | Serialize, Deserialize, JsonSchema | core/schema/ledger.rs:10-89 |
| `status::Day`, `Mark`, `Tone` | Serialize only (`Day.run` is `#[serde(skip)]`) | core/status.rs:17, 40, 48, 59 |
| `connections::Source`, `Group`, `Link` | Serialize only | core/connections/mod.rs:17, 37, 45 |
| `ledger::Entry` | Serialize only | core/ledger/mod.rs:58 |
| `config::WorkspaceConfig`, `Product`, `CrmConfig`, `CrmJoinKeys` | Serialize, Deserialize | core/config.rs:16, 22, 52, 67 |
| `provider::ModelProviderInfo`, `WireApi` | Serialize, Deserialize | core/provider.rs:23, 38 |
| `tools::Tool`, `ToolCall` | Serialize, Deserialize | core/tools.rs:13, 32 |
| `snapshot::AdsDailyRow` | Serialize, Deserialize | core/snapshot.rs:28 |

### Not `Serialize`

| Type | Current derives | Ref | Note |
|---|---|---|---|
| `agent::Ev` | none (not even Debug/Clone) | core/agent.rs:46 | The streaming payload. Tauri events need Serialize + Clone. |
| `agent::Usage` | Debug, Default, Clone, Copy, PartialEq, Eq | core/agent.rs:355 | Inside `Ev::TurnComplete`. |
| `agent::TurnResponse` | Debug, Default | core/agent.rs:362 | Internal; not needed over IPC. |
| `health::Health` | Debug, Clone | core/health.rs:35 | Landing input. All fields are serializable types. |
| `health::State` | Debug, Clone, Copy, PartialEq, Eq | core/health.rs:15 | |
| `crm::status::CrmHealth` | Debug, Clone, PartialEq | core/crm/status.rs:17 | `note()` gives a String. |
| `brief::ProductRow`, `Feasibility` | Debug, Clone | core/brief/mod.rs:27, 45 | `brief_json` (core/brief/print.rs:135) is a hand-built JSON view with different field names and CRM nulls. |
| `brief::ProductRowsAndFeasibility` | Clone | core/brief/mod.rs:325 | |
| `brief::HistoryRow` | Debug, Clone, Deserialize | core/brief/mod.rs:243 | |
| `funnel::FunnelRow` | Debug, Clone, Default | core/funnel/mod.rs:60 | No JSON view exists; MB_AGENT prints the text table (core/funnel/mod.rs:236-246). |
| `funnel::By` | Debug, Clone, Copy, PartialEq, Eq | core/funnel/mod.rs:25 | `parse`/`name` round-trip via &str. |
| `report::ReportOutput` | none | core/report/mod.rs:218 | Its three fields are all serializable. |
| `refresh::Progress<'a>` | none; borrows | core/refresh/mod.rs:26 | Needs an owned form (section 2, N). |
| `refresh::Options` | none | core/refresh/mod.rs:34 | Holds an `Arc<AtomicBool>`; input only. |
| `config::AppConfig` | Debug, Clone | core/config.rs:197 | |
| `snapshot::Snapshot` | Debug, Clone | core/snapshot.rs:95 | Holds raw lead PII (core/snapshot.rs:106-109); should not cross IPC whole. |
| `diagnose::Diagnosis`, `StageRow`, `AdRow`, `Window` | none / Copy only | core/diagnose/mod.rs:28, 43, 69, 99 | `diagnose::text` gives a String. |
| `session::SessionLog` | none | core/session/mod.rs:47 | A handle; stays Rust-side. |
| `error::Error` | Debug, thiserror | core/error.rs:7 | Tauri command errors must be Serialize; `kind()`, `reason()`, `fix()` already expose the three fields the binary prints as JSON (bin/main.rs:153-157). |
| `anyhow::Error` | - | session fns return `anyhow::Result` (core/session/mod.rs:18) | Needs mapping to a string at the command edge. |
| TUI-only: `refresh_flow::Ev`, `StreamEvent` | none | tui/refresh_flow.rs:19; tui/app.rs:82 | Not reusable from a crate that depends on core only. |

Adding `Serialize` to types under `core/schema/` is not needed (all have
it). Adding it elsewhere does not touch `tests/schema_contract.rs`, which
pins only the eight artifact structs listed at
crates/moneyball-core/tests/schema_contract.rs:13-16, 42-51.

---

## 5. Contract tests and what a fourth crate changes

All in `crates/moneyball-core/tests/arch_contract.rs`. Every rule iterates
a hardcoded crate list, so `crates/moneyball-desktop` is invisible to all
four until the lists are extended. All four scan `.rs` files only; the
webview frontend (HTML/CSS/JS/TS) is never scanned by any of them.

| Test | Rule today | Scans | Ref | For `moneyball-desktop` |
|---|---|---|---|---|
| `network_boundary` | The string `reqwest` may appear only in 12 allowed core files; the `moneyball` and `moneyball-tui` Cargo.toml files must not contain `reqwest` | `src/` of `["moneyball", "moneyball-core", "moneyball-tui"]` (line 61); manifests of `["moneyball", "moneyball-tui"]` (line 70) | 40-81 | Needs adding to both lists. The check is a substring match on `reqwest` only: a Tauri HTTP plugin, `tauri::http`, `ureq`, `hyper`, or a JS `fetch` in the webview would all pass unseen. The rule's intent (ARCHITECTURE.md:25-37: network in four core modules, nothing writes to Meta or the CRM) therefore needs a stronger needle set or a manifest deny-list for this crate, plus a webview CSP, since Tauri gives the frontend its own network path. |
| `ambient_state_only_in_config` | `"HOME"`, `"USERPROFILE"`, `current_dir(` may appear only in core/config.rs; comment lines exempt | `crates/moneyball-core/src` + `crates/moneyball-tui/src` (lines 93-95); the binary crate is deliberately not scanned ("only the binary resolves cwd", line 87) | 83-117 | A decision is required: the desktop crate is both a front-end and a binary. If scanned like the TUI it must route home through `config::home_dir` (core/config.rs:395) and never call `current_dir`. Tauri path APIs (`app.path().home_dir()`, `app_data_dir()`) do not match the needles and would bypass the rule silently. |
| `ascii_only_outside_comments` | Every non-comment line must be ASCII | `src/` and `examples/` of the three crates (lines 125-127) | 119-142 | Needs adding if desktop Rust strings are to be held to the rule. The rule's stated reason is terminal fonts (AGENTS.md:39-40; CLAUDE.md "ASCII only in TUI-facing strings we author"); a webview does not share it, and frontend files are outside the scan regardless. Strings moved into core (section 2: L, M, O2) stay bound by it; the TUI's `\u{00B7}` escapes (tui/stream_drain.rs:96) show the existing workaround. |
| `file_size_ratchet` | 400 lines per `.rs` file, no exceptions | `src/` of the three crates (line 151) | 144-165 | Needs adding. Tauri's default layout puts Rust under `src-tauri/src/`; the scan path is `crates/<name>/src`, so the crate's Rust must live at `crates/moneyball-desktop/src` or the path logic must change. `build.rs` at the crate root and generated code (`gen/`, `target/`) are outside `src/` and unscanned. Moving code into core (section 2) must respect the cap there: `core/config.rs` is at 399, `core/report/html.rs` 397, `tui/landing.rs` 395. |

Other tests and gates:

| Gate | Rule | Ref | For `moneyball-desktop` |
|---|---|---|---|
| `schema_contract::committed_schemas_match_the_structs` | JSON Schemas for 8 artifact structs must equal `docs/schemas/*.schema.json` | crates/moneyball-core/tests/schema_contract.rs:38-72 | Unaffected unless an artifact struct changes shape. Adding fields to `RunRecord`/`CreativeReport` for the UI would trip it (additive-only, regen with `MB_UPDATE_SCHEMAS=1`). |
| `workspace_config` | `workspace_id` is minted once at `WorkspaceConfig::load` and persisted | crates/moneyball-core/tests/workspace_config.rs:7-20 | The desktop must load config through `WorkspaceConfig::load` / `AppConfig::resolve*`, never deserialize config.json itself. Note `load` writes on first read (core/config.rs:132-135). |
| clippy `string_slice = "deny"` | No byte-index slicing | Cargo.toml:9-12 | Applies only if the new manifest has `[lints] workspace = true` (as the three existing manifests do). |
| Workspace membership | `members` list | Cargo.toml:3-7 | Must be added or `cargo test/clippy --workspace` skip it. `rust-version = "1.96"` (Cargo.toml:17) applies via `rust-version.workspace = true`. |
| Dependency rule | "Core has no ratatui/terminal dependency, ever"; `moneyball -> moneyball-tui -> moneyball-core` | ARCHITECTURE.md:21-23 | Prose only, no test. Nothing enforces that the desktop crate does not depend on `moneyball-tui`, or that core does not gain a tauri dependency. |
| Write paths | Writes only under `<workspace>/.moneyball/` and `~/.moneyball/`; the one exception is the launchd plist | AGENTS.md:56-64; ARCHITECTURE.md:41-53 | Prose only, no test. A Tauri app writes outside both by default: WebKit data and caches under `~/Library/WebKit/<bundle id>` and `~/Library/Caches/<bundle id>`, plus anything put in `app_data_dir()` (`~/Library/Application Support/<bundle id>`), window-state or store plugins. The rule needs an explicit exception or the app must avoid those APIs. |
| Secrets | Never in config.json, crm.toml, logs or errors | ARCHITECTURE.md:44-50; CLAUDE.md | Prose only. IPC adds a new surface: `ModelProviderInfo` is Serialize and safe (keys live in auth.json, core/config.rs:95-99), but nothing stops a command from returning `secrets::load_meta_token()` to the webview. |
| Definition of done | TestBackend render, `cargo install --path crates/moneyball` | AGENTS.md:18-26; ARCHITECTURE.md:185-193 | Steps 4 and 7 are TUI/CLI specific; no equivalent is defined for a desktop bundle. |

No CI workflow directory exists in the repo (`.github/` absent); the gates
run only through local `cargo test`.

---

## 6. Workspace and config resolution at startup, and Finder launch

### Today

Both the binary and the TUI call `AppConfig::resolve_optional`
(bin/main.rs:143, 170; tui/lib.rs:113), which never errors and returns
`workspace: None` when no config.json is found (core/config.rs:224-239).
`resolve_data_root` (core/config.rs:324-389) tries, in order:

1. `--data-root` CLI arg (325-329).
2. `MONEYBALL_DATA_ROOT` env var (331-335).
3. The current directory, if it holds `.moneyball/config.json` and is not
   HOME (341-347).
4. `data_root` key in `~/.moneyball/config.json` (349-362). No code in the
   repo writes this file; the only reference is this read.
5. Walk up from the cwd for a directory named `fin_campaign_analysis` or
   `data` (364-375).
6. `~/DEV/fin_campaign_analysis` or `~/fin_campaign_analysis` (377-384).
7. `<cwd>/moneyball-data` (387-388).

Then: `WorkspaceConfig::load(root)` reads `<root>/.moneyball/config.json`,
renames a legacy `<root>/moneyball/` dir, and mints + saves `workspace_id`
if absent (core/config.rs:108-137). `agent` comes from `MB_AGENT`
(core/config.rs:230-232). `date` comes only from the `--date` flag
(bin/main.rs:22-24, 170). With no workspace the TUI opens the setup wizard
(tui/app.rs:156-160) and sessions go to `~/.moneyball/sessions/`
(core/config.rs:253-255; core/session/mod.rs:137-140). Secrets are global:
`~/.moneyball/auth.json` via `config::home_dir` (core/secrets.rs:50-58).

### What a Finder-launched GUI gets wrong

Platform facts assumed here, not verified in this repo: a Finder/launchd
app starts with cwd `/`, with `HOME`, `USER`, `TMPDIR` and a minimal
`PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), and without anything exported
from shell profiles.

| # | Problem | Why | Ref |
|---|---|---|---|
| 1 | The workspace is not found | Steps 1-2 have no input; step 3 looks at `/.moneyball/config.json`; step 4's file is never written by moneyball; so a normal user falls through to step 7 = `/moneyball-data`, which does not exist and sits on the read-only system volume. Result: `has_workspace() == false` although the user has a working workspace elsewhere. | core/config.rs:341-347, 349-362, 387-388 |
| 2 | A wrong directory can be adopted | Step 5 walks up from `/` and accepts `/data` or `/fin_campaign_analysis` if either exists; step 6 picks `~/DEV/fin_campaign_analysis` on this developer machine regardless of which workspace the user last used. | core/config.rs:364-375, 377-384 |
| 3 | No memory of the last workspace | The TUI relies on cwd for this; there is no "recent workspaces" record. The setup wizard saves config into the workspace only (tui/setup/steps.rs:148), never the global pointer of step 4. | core/config.rs:349-362 |
| 4 | First run has no path to a workspace | `workspace: None` means setup, and setup exists only as TUI code (section 2, X). Its default path is the resolved root, here `/moneyball-data`. | tui/app.rs:156-160; tui/setup/mod.rs:89 |
| 5 | Sessions land in the global dir | With no workspace, `sessions_root()` is None, so chats are written to `~/.moneyball/sessions/` and do not appear in the workspace's list later (only id lookups fall back). | core/config.rs:253-255; core/session/mod.rs:137-140, 150-160 |
| 6 | LLM key from the environment is missing | `api_key` falls back to the provider's `env_key` variable when auth.json has none. A user who exports the key in a shell profile gets `LlmAuth("no API key for provider ...")` in the GUI while the TUI works. | core/provider.rs:79-92; core/llm.rs:166-168 |
| 7 | Env-sourced HTTP headers are dropped silently | `env_http_headers` values are read from the environment and skipped when unset, with no error. | core/llm.rs:294-298 |
| 8 | crm.toml `env:VAR` references fail | Resolved with `std::env::var`; refresh's CRM step fails with "env var X is not set". `secret:<name>` references are unaffected (auth.json). | core/crm/refs.rs:66-68 |
| 9 | `MONEYBALL_DATA_ROOT` set in a shell profile is ignored | Same cause; step 2 is skipped. | core/config.rs:331 |
| 10 | No way to pin a date | `AppConfig.date` is fed only by `--date`; `State::Pinned` and every `snap_for(cfg.date)` call depend on it. A GUI must set the field itself. | core/config.rs:200; core/health.rs:73-76, 87 |
| 11 | "Today" and staleness are computed at call time | `Health::of`, `staleness_warning`, `refresh::run` and `data_day_label` each read `Local::now()`. A window left open overnight shows yesterday's health until something recomputes it; the TUI recomputes on `/brief` and after a refresh (tui/app.rs:272-302). | core/health.rs:55; core/brief/mod.rs:342; core/refresh/mod.rs:55; core/refresh/show.rs:13-17 |
| 12 | stdout/stderr go nowhere | Core prints in the headless runners (`brief::run`, `funnel::run`, `report::run`) and in `crm::fetch` when `!cfg.agent`. `refresh::run` already forces `agent: true` for its CRM step; a direct `fetch_crm` call from the GUI would print into the void. | core/crm/fetch.rs:115, 179, 212; core/refresh/mod.rs:211-214 |
| 13 | The schedule would point at the wrong executable | `schedule::plan(cfg, exe, at)` bakes `exe` into the plist with args `--data-root <root> refresh --scheduled`; callers pass `current_exe()`. From the desktop app that is the bundle binary, which has no such CLI. Not a v1 surface, but the landing reads `schedule::installed`. | core/schedule.rs:43-60, 91-104, 109; tui/schedule_view.rs:190; bin/schedule_cli.rs:27 |
| 14 | Text that tells the user to type a command | Fixes and notes are phrased as slash commands or "in a shell" (`/connect crm`, `/setup meta`, `moneyball crm secret <name>`); a Finder user may have neither a TUI nor the CLI on PATH. | core/crm/status.rs:58; core/refresh/mod.rs:206; core/crm/refs.rs:59; tui/widgets.rs:166-178 |
| 15 | macOS privacy prompts (not from code) | A workspace under `~/Documents`, `~/Desktop` or `~/Downloads` triggers a TCC prompt for a GUI app; the TUI inherits the terminal's grant. | - |

What works unchanged from Finder: `config::home_dir` (HOME is set), hence
auth.json, the global sessions dir, bug reports and the plist path; the
per-workspace refresh lock, so the desktop app, the CLI and the launchd
job cannot refresh the same workspace at once (core/runs.rs:47-58); the
auth.json write lock (core/secrets.rs:77-90); `open` for the report, which
is on the default PATH (tui/refresh_flow.rs:208-222).
