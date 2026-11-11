# core-data: API note

Branch `desktop-core-data`. Data loading and refresh for a second
front-end (seam doc section 2 items N and Q, sections 4 and 6). Paths are
under `crates/moneyball-core/src/`. Nothing under `schema/` changed; core
gained no dependency.

## New

| Item | Signature | A non-TUI caller |
|---|---|---|
| `workspace::open` (workspace.rs) | `fn open(dir: &Path, date: Option<&str>) -> Result<AppConfig>` | `workspace_open(path)`: opens exactly `dir`. No flag, env, cwd, walk-up, home paths or `moneyball-data` fallback. `Err(Error::NoWorkspaceConfig)` when `dir` has no `.moneyball/config.json` - show it, there is no setup in v1. `date` pins a snapshot (problem 10). Always `workspace: Some`, so sessions go to the workspace (problem 5). `agent` is false; set the field if you call something that prints. |
| `workspace::remember` | `fn remember(dir: &Path) -> Result<()>` | Call after a successful `open`. Writes its own key, `last_workspace`, into `~/.moneyball/config.json`, atomically, keeping every other key. Refuses (Err, file untouched) when that file is not a JSON object. |
| `workspace::last` | `fn last() -> Option<PathBuf>` | `workspace_current` at launch: `last().map(|d| open(&d, None))`. None on a first run. The directory may be gone: `open` says so. |
| `refresh::Ev` (refresh/worker.rs) | `enum Ev { Begin { step: &'static str }, Done(Box<RunStep>), Finished { run: Box<RunRecord>, ms: u64 }, CouldNotStart { error: String } }`; Debug, Clone, Serialize, `tag = "type"`, snake_case | Forward each to the webview channel as is. JSON: `{"type":"begin","step":"meta"}`, `{"type":"done", ...RunStep fields}`, `{"type":"finished","run":{...},"ms":1234}`, `{"type":"could_not_start","error":"..."}`. Exactly one of the last two ends a run; a channel that closes without either means the worker died (the TUI then says "its run record keeps the steps that finished"). |
| `refresh::spawn` | `fn spawn(cfg: AppConfig, meta_token: Option<String>, opts: Options) -> (Receiver<Ev>, Arc<AtomicBool>)` | `refresh_start`: pass `secrets::load_meta_token()` (the caller resolves the token, as for `refresh::run`; never send it over IPC) and `Options { trigger: Trigger::Manual, force, days: 28, cancel: None }`. Keep the returned flag; `refresh_cancel` is `flag.store(true, SeqCst)` and takes effect after the step in flight. A flag passed in `opts.cancel` is used and returned instead. |
| `brief::load` (brief/load.rs) | `fn load(cfg: &AppConfig) -> Loaded` | One call for `landing` and `brief`. Never fails. Reads the clock (`Health::of`), so call it again after a refresh and when the window regains focus. |
| `brief::Loaded` | `struct Loaded { health: Health, brief: Option<ProductRowsAndFeasibility>, crm_note: Option<String>, fault: Option<Error> }`; Serialize | `brief: None, fault: None` = no snapshot yet (not a fault; `health.state` is `no_data`). `brief: None, fault: Some` = a snapshot exists but would not load. The snapshot date is `brief.snapshot_date`. As in the TUI before, any `snap_for` error (a malformed pinned date included) reads as "no snapshot". |

## Changed

| Item | Change | A non-TUI caller |
|---|---|---|
| `Error` (error.rs) | `impl Serialize`: exactly `{"kind", "reason", "fix"}` from `kind()`, `reason()`, `fix()`; `fix` is null when there is none | A command can return `Result<T, moneyball_core::Error>`. bin/main.rs is untouched (its MB_AGENT JSON calls the second field `message`). |
| `brief::ProductRowsAndFeasibility` | `impl Serialize` (brief/wire.rs) | Return it from `brief`. Rust field names: `rows[]`, `feasibility`, `snapshot_date`, `has_crm`. |
| `funnel::FunnelRow` | `impl Serialize` (funnel/wire.rs) | Return `Vec<FunnelRow>` from `funnel`. Rust field names. |
| `funnel::By` | derives Serialize: `"campaign"`, `"adset"`, `"ad"` (same as `By::name`) | Input still goes through `By::parse(&str)`. |
| `crm::status::CrmHealth` | derives Serialize, `tag = "kind"`: `{"kind":"no_spec"}`, `{"kind":"no_data","last":..,"failed":[why, fix]}`, `{"kind":"fresh"}` | `failed` is a 2-element array or null. |
| `report::ReportOutput` | derives Serialize | `report`, `json_path`, `html_path`. |
| `health::Health`, `health::State` | derive Serialize; `State` is `tag = "kind"`: `{"kind":"fresh"}`, `{"kind":"partial","have":1,"of":2}` | Needed for `Loaded`. Not in my item list, but PLAN.md and seam section 4 name them for core-data. |

Not Serialize, on purpose: `Snapshot` (lead PII), `AppConfig`, and
`brief::ProductRow` / `brief::Feasibility` on their own (next section).

## "No CRM data" is null, never zero

The Rust structs hold plain zeros when the snapshot has no CRM data, and
I did not change their fields (the TUI and the tool text read them). So
the rule lives in the serializers, each a private derived struct whose
CRM fields are `Option`:

- Brief: only the parent knows `has_crm`, so only the parent is
  Serialize. `ProductRow` and `Feasibility` have no Serialize impl at
  all: a row cannot be serialized without its `has_crm`, and that is
  checked by the compiler, not by convention. This is the one place I
  did not follow "derive on ProductRow and Feasibility" to the letter.
  With `has_crm == false`: `rows[].l7d, q7d, q_per_day, gap, rs_per_q,
  l_to_q` and `feasibility.tot_q_per_day, cur_rpq, required_at_cur,
  required_at_best, best_rpq` are null. Spend, Meta leads, CPL, goals
  and `open_debt` stay.
- Funnel: a row carries its own marker - `kill` is None exactly when the
  snapshot had no CRM data (funnel/mod.rs) - so `FunnelRow` is Serialize
  alone. With `kill == None`: `l, q, v, b, rs_per_q, l_to_q, immature`
  and `kill` are null.
- Tests: `brief::wire::tests`, `funnel::wire::tests`, and
  `brief::load::tests` (end to end from a snapshot on disk).

Because these are hand-written impls over private structs, a
derive-based TypeScript generator will not see the nullable fields:
wave 2 must type them by hand or generate from the wire structs.

## TUI

- `refresh_flow.rs`: its `Ev` enum and worker thread are gone; it calls
  `refresh::spawn` and keeps the cell, the notes and the drain. The Meta
  token is now read from auth.json on the UI thread just before the
  spawn instead of inside the worker (one small file read).
- `app.rs`: `load_brief` calls `brief::load`; `snapshot_load` and
  `mod r` are deleted (and the `snapshot_load` re-export in lib.rs).

## What the lead and the others should know

- `workspace::remember` / `last` use the `last_workspace` key, not
  `data_root`. `config.rs` is byte-identical to before this branch:
  step 4 still reads `data_root`, nothing writes it, and the CLI and TUI
  resolve exactly as they did (an empty directory still gets setup).
- `refresh/mod.rs` gained `mod worker;` and `pub use worker::{spawn, Ev};`
  next to `mod show;` - core-words adds its words module in the same
  lines.
- Render examples: byte-identical except `render_chat` line 30, a
  wall-clock time (`00:44:53` vs `00:52:12`) printed by the example.
