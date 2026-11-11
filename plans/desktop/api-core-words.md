# API note: core-words (seam items L, M, M2, O2)

Branch `desktop-core-words`. Everything below is in `moneyball-core`,
unstyled, and `Serialize` (snake_case enums). The TUI now only lays these
out and colours them; its output is unchanged.

## `moneyball_core::words` (new)

| Item | Signature | Non-TUI use |
|---|---|---|
| `Tint` | `enum { Plain, Good, Warn, Bad }` | Map to your own colour / icon. The TUI's "! " and cross marks are no longer in the text. Not `status::Tone` (that one is a /status row's state). |
| `Command` | `struct { command: String, does: String }`; `Command::new(&str, &str)`, `Command::fix(&str)` | Something the user types, kept out of the prose. Render as a button or "In Terminal:" + copy. `does` may be empty. |
| `fix_does` | `fn(fix: &str) -> &'static str` | What a fix string (`RunStep.fix`, `connections::Source.fix`) does; `Command::fix` wraps it. |

## `moneyball_core::landing` (new; L, M)

| Item | Signature | Non-TUI use |
|---|---|---|
| `landing` | `fn(cfg: &AppConfig, h: &Health, scheduled: bool) -> Landing` | The whole `landing` command: pass `Health::of(..)` and `schedule::installed(cfg).is_some()`. File reads only. |
| `Landing` | `{ headline: Option<Headline>, details: Vec<String>, fixes: Vec<Command>, next: Option<Command>, schedule: Option<Command>, day: Option<DayBlock> }` | Render top to bottom in field order. `headline` is None when fresh; `day` is None for no-data / stale / failed. |
| `Headline` | `{ tint: Tint, text: String, rest: String }` | `text` strong in the tint, `rest` (may be empty) quieter after it. |
| `DayBlock` | `{ day: String, refreshed: Option<String>, report: DayReport }` | `day` = "Thu 08 Oct"; `refreshed` = "07:30 this morning (scheduled, 1m 21s)", label it yourself. |
| `DayReport` | `enum, tag "kind": Missing { note: String, next: Command } / Written { numbers: DayNumbers, versus: Option<Versus>, path: PathBuf }` | `path` is the day's HTML report (what the TUI's ctrl+o opens). |
| `DayNumbers` | `{ numbers: Vec<Number>, note: Option<String> }`; `phrases(&self) -> Vec<&str>` | Stat tiles from `numbers`; `note` = "no CRM in this report". `phrases()` gives one-line wording to join with your separator. |
| `Number` | `{ label: String, value: String, phrase: String }` | label "spend", value "Rs.51,234", phrase "spend Rs.51,234". |
| `Versus` | `{ day: String, changes: Vec<Change> }` | "vs <day>" row. |
| `Change` | `{ label: String, by: i64, text: String, tint: Tint }` | `text` = "+2" / "-Rs.3,532"; tint Good/Bad is for the business (cost down = Good). |
| `day_numbers` | `fn(report: &CreativeReport) -> DayNumbers` | The same numbers after a refresh, from the run's report json. |

## `moneyball_core::refresh` (added in `refresh/finish.rs`; O2)

| Item | Signature | Non-TUI use |
|---|---|---|
| `cancelled` | `fn(s: &RunStep) -> bool` | Whether a skipped step was a cancel. |
| `gaps` | `fn(run: &RunRecord) -> String` | Header words for a degraded run ("finished without CRM"). |
| `verdict` | `fn(run: &RunRecord, last_good: Option<&str>, day: &str) -> Option<RunVerdict>` | The sentence under the steps; None for a clean run. `last_good` = data day shown before the run, `day` = the day the run fills. |
| `RunVerdict` | `{ tint: Tint, headline: String, rest: String, next: Option<Command> }` | `next` is set for cancel (`/refresh`, "runs the rest") and a failed report (`/report`, "builds one from it"); the TUI joins it as "rest; command does." |
| `fixes` | `fn(run: &RunRecord) -> Vec<Command>` | The steps' fixes with what each does. |
| `rerun` | `fn(run: &RunRecord) -> Command` | The refresh to offer after the fixes; `does` is empty when Meta itself failed. |

## Changed

- `health::Health`: `+ Serialize`. `health::State`: `+ Serialize`, tagged
  `{"kind": "partial", "have": 4, "of": 5}` / `{"kind": "fresh"}`.
- TUI: `widgets::fix_does` and `refresh_cell/words.rs` are gone;
  `widgets::day_numbers` stays as a one-line join of core's phrases
  (refresh_flow.rs, off limits to this helper, calls it).

## Known, left as is

- Command strings are still slash commands and shell commands
  (`/refresh`, `/connect crm`, `moneyball crm secret <name>`), and
  `fix_does` still says "in a shell". They are now isolated in `Command`
  for the desktop to translate.
- The pinned headline still reads "Viewing <day> (--date)."
