# core-agent: API note

Branch `desktop-core-agent`. Everything the advisor chat needs now lives in
`moneyball_core::agent` and `moneyball_core::session`; the TUI keeps cells,
keys and its own wording. Seam doc items A B C C2 C3 D E F H J2 + section 3.

## The session driver - `agent::Session` (agent/driver.rs)

One conversation: history, session file, cancel flag, one-turn guard. A plain
struct owned by the front-end (put it in `tauri::State<Mutex<_>>`).

| Item | Signature | How a non-TUI caller uses it |
|---|---|---|
| `Session::new` | `(log: Option<SessionLog>, history: Vec<Item>) -> Session` | New chat: `Session::new(SessionLog::create(cfg.data_root.clone(), cfg.sessions_root()).ok(), vec![])`. Reopen: `let (log, items) = SessionLog::open(id, root)?; Session::new(Some(log), items)`. `None` log = not persisted. |
| `Session::history` | `(&self) -> &[Item]` | Feed to `agent::view_items` for a snapshot after a webview reload. |
| `Session::id` | `(&self) -> Option<&str>` | The session file id; None when not persisted. |
| `Session::turn_active` | `(&self) -> bool` | True from `start_turn` until a final event is applied. |
| `Session::start_turn` | `(&mut self, cfg: &AppConfig, front: &prompt::FrontEnd, system: &str, turn: Vec<Item>) -> Result<Turn, TurnRefused>` | `front` is the same `FrontEnd` the state block got; its `say` words every step the tools name (no-snapshot error, recommend result, CRM note, diagnose). `turn` is normally `vec![Item::User { text }]`. Records it, spawns the worker (provider/model lookup, tool list, executor, `run_turn`), returns at once. `system` = a persona const + `"\n\n"` + `prompt::app_state_block(..)`; the recommend rule is appended inside. A refusal records nothing. |
| `Turn` | `struct { events: Receiver<Ev>, save_failed: Option<SaveFailed> }` | Forwarder thread: `for ev in turn.events { session.apply(&ev); channel.send(ev) }`, then `session.stream_closed()`. `save_failed` = the turn runs but its items missed the file; show once. |
| `TurnRefused` | `enum { NoModel, UnknownProvider { provider }, Busy }`; Debug, Clone, PartialEq, Eq, thiserror, Serialize (`{"type":"no_model"}`, `{"type":"unknown_provider","provider":".."}`, `{"type":"busy"}`) | The typed "did not start" result. Its `Display` is neutral; each front-end writes its own sentence. |
| `Session::apply` | `(&mut self, ev: &Ev) -> Result<(), SaveFailed>` | Call for EVERY event, in order. Records `ItemDone` items (history + file) and frees the guard on `TurnComplete` / `TurnAborted` / `Failed`. |
| `Session::stream_closed` | `(&mut self)` | Call when the receiver disconnects. Harmless after a normal end; after a dead worker it is what frees the session (then tell the user to resend - never a silent dead turn). |
| `Session::interrupt` | `(&self) -> bool` | `turn_cancel`. Sets the cancel flag if a turn is in flight; the stream then ends with `Ev::TurnAborted`. |
| `Session::record` | `(&mut self, item: Item) -> Result<(), SaveFailed>` | Append an item outside a turn (the TUI's bug-report marker). The item is in the history even on Err. |
| `Session::typed_tool_turn` | `(&self, typed: &str, tool: &str, args: Value, output: String) -> Vec<Item>` | The `turn` for a button that stands for a tool ("Brief" pressed): user item + tool call + its output, so the model does not call the tool again. |
| `SaveFailed` | `struct SaveFailed(pub String)`; Display `session save failed: <why>`; Serialize (a string) | Show once; the chat goes on. |
| `looks_like_context_overflow` | `(error: &str) -> bool` | On `Ev::Failed { error }`: true = offer a new session (the TUI's hint names `/clear`). |

## Events - `agent::Ev`, `agent::Usage` (agent.rs)

`Ev` now derives Debug, Clone, Serialize, tagged `type`, snake_case.
**Shape change:** the two tuple variants became struct variants so the tag
works: `AssistantDelta { text }` (was `AssistantDelta(String)`) and
`ItemDone { item }` (was `ItemDone(Item)`). `Usage` gained Serialize.

```json
{"type":"assistant_delta","text":"hi"}
{"type":"assistant_done","text":"..."}
{"type":"tool_begin","call_id":"c1","name":"funnel","args":{}}
{"type":"tool_end","call_id":"c1","output":"...","ok":true}
{"type":"item_done","item":{"type":"user","text":"q"}}
{"type":"turn_complete","ms":5,"provider":"p","usage":{"input":10,"output":2}}
{"type":"turn_aborted"}
{"type":"failed","error":"..."}
```

## View projection - `agent::view_items` (agent/view.rs)

| Item | Signature | Use |
|---|---|---|
| `view_items` | `(history: &[Item]) -> Vec<ViewItem>` | Reopen / reload: `view_items(session.history())`. The TUI replay calls it. |
| `ViewItem` | `enum { User { text }, Assistant { text }, Tool { call_id, name, args, status, output }, TurnInterrupted, BugReportFiled }`; Debug, Clone, PartialEq, Serialize (tagged `type`, snake_case) | One row each. The two markers are notes ("(turn interrupted)", "(bug report filed)" in the TUI), never user messages. |
| `ToolStatus` | `enum { Pending, Ok, Failed }`; Serialize snake_case | `Pending` = no output recorded (running live, cut by an interrupt in a saved session); `output` is empty then. |

## Prompts and app state - `agent::prompt` (agent/prompt.rs)

| Item | Signature | Use |
|---|---|---|
| `AGENT_SYSTEM_PROMPT`, `BRIEF_SYSTEM_PROMPT`, `FUNNEL_SYSTEM_PROMPT`, `DIAGNOSE_SYSTEM_PROMPT` | `pub const &str` | The persona half of `system`. Free chat uses `AGENT_SYSTEM_PROMPT`. |
| `FrontEnd` | `struct FrontEnd<'a> { commands: &'a [&'a str], output_style: &'a str, say: Say }` | One per front-end, passed to both `app_state_block` and `start_turn`. Desktop v1: `commands: &[]`, its own `output_style` (no trailing newline), its own `say`. |
| `Say` | `type Say = fn(&str) -> String` | The one translation hook. Core names every step the way the TUI types it - a slash command with its args (`/refresh`, `/setup`, `/brief`, `/connect crm`, `/ledger r7 applied`) or a shell command (`moneyball crm secret api_key`) - and passes it through `say` before the model reads it. Return how this front-end's user does that step; return the input unchanged only if it can be typed here as is (the TUI's `say` is the identity). A plain fn (not a closure) so the worker thread can carry it. Do not build it on `connections::fix_in_shell` alone: its fallback is "open moneyball and run /x", which still names a slash command. |
| `app_state_block` | `(cfg: &AppConfig, snap_date: Option<&str>, crm_note: Option<&str>, front: &FrontEnd) -> String` | `snap_date` / `crm_note` = the loaded snapshot's date and `CrmHealth::note()` (core-data's brief loader has both). Empty `commands` prints `slash commands: none (this app has none - never suggest one)`. |

## Tool text - `agent::tool_text` (agent/tool_text.rs)

| Item | Signature | Use |
|---|---|---|
| `format_brief_as_lines` | `(b: &ProductRowsAndFeasibility) -> Vec<String>` | The brief as plain lines (what the brief tool row shows). |
| `brief_text` | `(b: &ProductRowsAndFeasibility, crm_note: Option<&str>) -> String` | The `output` for `typed_tool_turn(.., "brief", ..)`. |
| `funnel_output` | `(cfg: &AppConfig, snap: &Snapshot, product: &str, by: By, say: &dyn Fn(&str) -> String) -> String` | Same, for the funnel tool. |
| `diagnose_output` | `(cfg: &AppConfig, snap: &Snapshot, product: &str, say: &dyn Fn(&str) -> String) -> String` | Same, for the diagnose tool. |

The executor (`SnapshotTools`) and the wire tool list are private to
`agent` (agent/exec.rs); `start_turn` is their only caller.

## Session list - `session::summaries` (session/summary.rs)

| Item | Signature | Use |
|---|---|---|
| `summaries` | `(root: Option<&Path>) -> anyhow::Result<Vec<SessionSummary>>` | `sessions_list`: `session::summaries(cfg.sessions_root())`. Most recently modified first; unparseable files left out. Map the anyhow error to a string at the command edge. |
| `SessionSummary` | `struct { id: String, started_at: DateTime<Utc>, modified_at: DateTime<Utc>, item_count: usize, preview: Option<String> }`; Debug, Clone, Serialize | `preview` = first real user text, one line, at most 200 bytes + "..."; markers never count. `item_count` counts every item. File format unchanged. |

## Wording hooks outside `agent` (third round)

| Item | Signature | Use |
|---|---|---|
| `CrmHealth::note_for` | `(&self, say: &dyn Fn(&str) -> String) -> Option<String>` (crm/status.rs) | The CRM sentence with each fix said the front-end's way. Desktop: `CrmHealth::of(cfg, &snap).note_for(&front.say)` is what goes into `app_state_block`'s `crm_note` and onto the screen. A fix `say` returns unchanged gets its shell form beside it ("/refresh (moneyball refresh in a shell)"); a translated one stands alone. |
| `CrmHealth::note` | unchanged signature | Now `note_for` with the identity; byte-identical in all seven states (tested against the old strings, and the test was run against the old code). |
| `diagnose::text_for` | `(d: &Diagnosis, say: &dyn Fn(&str) -> String) -> String` (diagnose/print.rs) | `diagnose::text` with the "next refresh" step said the front-end's way; `text` is `text_for` with the identity. |

`refresh` / `setup` / `brief` / `mark_applied` are gone from `FrontEnd`:
all four are now `say("/refresh")`, `say("/setup")`, `say("/brief")` and
`say("/ledger <id> applied")` (the word "type" before the last stays in
core), and the TUI text did not change.

## Known gaps for the lead

- Closed: with a desktop-style `say`, the system prompt, the tools' own
  sentences, the CRM note in every state, and the brief / funnel /
  diagnose tool outputs carry no slash command
  (agent/exec_tests.rs `a_front_end_without_commands_gets_no_slash_command`).
- `brief::load`'s `Loaded.crm_note` (and `crm_note` in IPC.md) is still
  `note()`, i.e. TUI wording with slash commands. The desktop must build
  its note with `note_for` instead of passing that one to the state block
  or the screen.
- Two sentences read oddly once translated, because the TUI words around
  the step stay fixed: "the next the Refresh button pulls it" (diagnose)
  and "<say(/brief)> self-heals by running <say(/refresh)>" (state block,
  no snapshot). Choose `say` outputs that sit well after "run" / "the
  next", or reword the TUI text in a later round.
- `BRIEF_`, `FUNNEL_` and `DIAGNOSE_SYSTEM_PROMPT` name `/brief` and
  `/diagnose` (said in their doc comments); the desktop v1 uses
  `AGENT_SYSTEM_PROMPT` only.
- `ARCHITECTURE.md` 6b still describes `Op` and "the UI records each item";
  the truth is `Session::start_turn / apply / interrupt` (wave 3 doc fix).
