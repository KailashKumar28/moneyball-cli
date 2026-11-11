# Desktop front-end - reference research (codex-rs, pi-mono)

Source read 2026-11-12: openai/codex @ 54b90f9, badlogic/pi-mono @ 4b519d3.
Paths are repo-relative. Two findings that change the brief:

- **pi-mono no longer has `packages/web-ui`.** GUI work moved to `apps/p2`
  (README: "EXPERIMENTAL ... will be gone soon ... do not build on it") and
  `apps/p2-mac` (AppKit + WKWebView over a local p2 HTTP server). The stable pi
  GUI seam is `packages/agent` events + the `coding-agent` SDK/RPC.
- **codex has an in-process app-server** (`app-server/src/in_process.rs`) used
  by its own TUI and exec: the closest analogue to our plan, and its header
  states what keeping the protocol costs when there is no process boundary.

## 1. How each reference separates GUI from core

### codex

| Layer | Path | Role |
|---|---|---|
| Core | `codex-rs/core` (`CodexThread::next_event()` -> `EventMsg`) | agent loop, emits internal events |
| Protocol | `codex-rs/app-server-protocol/src/protocol/{common.rs,v2/*.rs}` | typed JSON-RPC: `ClientRequest`, `ServerNotification`, `ServerRequest` (macro tables at `common.rs:523`, `:1899`, `:2052`) |
| Server | `codex-rs/app-server/src/message_processor.rs`, `request_processors/` | request dispatch; one processor per concern |
| Per-thread listener | `app-server/src/request_processors/thread_lifecycle.rs:330,532` | one task per loaded thread loops `conversation.next_event()`, maps to notifications, fans out to subscribed connections |
| Event mapping | `app-server-protocol/src/protocol/event_mapping.rs` (1:1), `app-server/src/bespoke_event_handling.rs` (stateful) | core `EventMsg` -> v2 notification |
| In-process host | `app-server/src/in_process.rs` | same `MessageProcessor`, bounded in-memory channels instead of a socket |
| Client facade | `codex-rs/app-server-client/src/lib.rs` | `AppServerClient::{InProcess,Remote}`; `request_typed`, `next_event`, `shutdown` |

Key property: the GUI never sees core `EventMsg` or the rollout format, only a
projection (`ThreadItem`) built by one builder (`thread_history.rs`) for both
live and on-disk history, so the two cannot diverge.

### pi-mono

| Layer | Path | Role |
|---|---|---|
| Loop + events | `packages/agent/src/{agent-loop.ts,agent.ts,types.ts}` | `Agent.subscribe(listener)` (`agent.ts:275`); `AgentEvent` union (`types.ts:528`) |
| Session layer | `packages/coding-agent/src/core/agent-session.ts:210` | `AgentSessionEvent` = `AgentEvent` + queue/compaction/retry/`entry_appended` |
| RPC mode | `packages/coding-agent/src/modes/rpc/{rpc-mode.ts,rpc-types.ts,jsonl.ts,rpc-client.ts}` | JSONL over stdin/stdout for other-language hosts |
| Persistence | `packages/coding-agent/src/core/session-manager.ts`, `docs/session-format.md` | JSONL entries, tree via `parentId` |

Key property: one subscribe call is the whole seam; RPC mode is glue over it
(`rpc-mode.ts:362`: `session.subscribe(e => output(toJsonEvent(e)))`). pi's docs
split the cases: in-process -> methods + events; child process -> JSONL.

## 2. Event and item lifecycle a GUI consumes

### codex (wire names from `common.rs`; shapes from `v2/item.rs`, `v2/turn.rs`, `v2/thread_data.rs`)

```
client -> server                      server -> client (notifications)
initialize / initialized              thread/started, thread/status/changed
thread/start | thread/resume          turn/started        { threadId, turn }
turn/start   { threadId, input }      item/started        { threadId, turnId, item, startedAtMs }
turn/steer   { expectedTurnId, .. }   item/agentMessage/delta          { threadId, turnId, itemId, delta }
turn/interrupt { threadId, turnId }   item/commandExecution/outputDelta { .., itemId, delta }
thread/unsubscribe                    item/reasoning/*Delta, item/mcpToolCall/progress
                                      item/completed      { threadId, turnId, item, completedAtMs }
                                      error               { error, willRetry, threadId, turnId }
                                      turn/completed      { threadId, turn }
```

```rust
struct Turn { id, items: Vec<ThreadItem>, items_view: NotLoaded|Summary|Full,
              status: InProgress|Completed|Interrupted|Failed,
              error: Option<TurnError>, started_at, completed_at, duration_ms }
struct ThreadItem { payload: ThreadItemPayload, started_at_ms, completed_at_ms } // flat on wire
enum ThreadItemPayload {            // #[serde(tag = "type")], every variant has `id`
  UserMessage{id, content}, AgentMessage{id, text, phase}, Reasoning{id, summary, content},
  CommandExecution{id, command, status, aggregated_output, exit_code, duration_ms},
  McpToolCall{id, server, tool, status, arguments, result, error, duration_ms},
  DynamicToolCall{id, tool, arguments, status, content_items, success, duration_ms},
  FileChange{..}, Plan{..}, ContextCompaction{id}, .. }
enum ThreadStatus { NotLoaded, Idle, SystemError, Active{active_flags} }
```

Rules the GUI relies on:

- **Item id is the join key.** `item/started` creates the row, deltas append by
  `itemId`, `item/completed` replaces it with the authoritative item. A tool
  call is ONE item whose `status` moves InProgress -> Completed/Failed.
- **Deltas are disposable.** `in_process.rs:114` lists what must be delivered
  (`TurnCompleted` and a few others); text deltas may drop under saturation,
  the client gets `Lagged{skipped}` and repairs from `item/completed`.
- **Terminal events are thin.** `turn/started` has `items: []` (NotLoaded,
  `bespoke_event_handling.rs:163-180`); `turn/completed` at most the last agent
  message. `turn/start` returns the `Turn` (id for interrupt) immediately.

### pi (`packages/agent/src/types.ts:528`, `packages/ai/src/types.ts:831`)

```ts
type AgentEvent =
  | { type: "agent_start" } | { type: "agent_end"; messages }
  | { type: "turn_start" }  | { type: "turn_end"; message; toolResults }
  | { type: "message_start"; message }
  | { type: "message_update"; message; assistantMessageEvent }  // message = full partial so far
  | { type: "message_end"; message }
  | { type: "tool_execution_start";  toolCallId; toolName; args }
  | { type: "tool_execution_update"; toolCallId; toolName; args; partialResult }
  | { type: "tool_execution_end";    toolCallId; toolName; result; isError; durationMs? }
type AssistantMessageEvent = text_start|text_delta|text_end | thinking_* | toolcall_* | done | error
```

- `message_update` carries the delta AND the accumulated partial; a GUI can
  render from the snapshot and ignore deltas.
- Core holds render state itself: `AgentState { isStreaming, streamingMessage,
  pendingToolCalls, errorMessage, messages }`, reduced in `agent.ts:575` before
  listeners run. Listeners are awaited - that is how RPC mode applies stdout
  backpressure (`rpc-mode.ts:368`).
- Session layer adds `agent_settled`, `queue_update`, `entry_appended{entry}`.

Convergence: begin / delta / end keyed by a stable id, end carries the complete
payload. Our `Ev` already has this shape.

## 3. Listing, resuming, rendering on reopen

### codex

- **List**: `thread/list { cursor, limit, sortKey, archived, cwd, searchTerm }`
  -> `{ data: Vec<Thread>, nextCursor }`; `Thread { id, preview, name, createdAt,
  updatedAt, status, cwd, turns: [] }`. Backed by a SQLite state DB, not a scan.
  `thread/read`, `thread/turns/list`, `thread/items/list` page without loading.
- **Resume**: `thread/resume { threadId }` -> `{ thread (with turns), model, .. }`
  and subscribes the connection to live notifications.
- **Render**: `build_turns_from_rollout_items` / `ThreadHistoryBuilder`
  (`thread_history.rs:94,291`) replays rollout lines into the same `ThreadItem`s
  the live stream emits. The current rollout format persists canonical
  `ItemCompleted` lines, so projection is stateless per line
  (`thread_history_projection.rs:21`: turn start/complete/abort -> turn
  metadata, ItemCompleted -> item, every other line kind ignored).
- **Reconnect to a running thread**: resume goes through the listener task as
  `ThreadListenerCommand::SendThreadResumeResponse` (`thread_state.rs:66`):
  history + `active_turn_snapshot()` + subscribe, atomically, so no event falls
  between snapshot and stream. The server keeps a live builder per thread for
  this (`thread_state.rs:236`).

### pi

- **List**: `SessionManager.list(cwd)` / `listAll()` (`session-manager.ts:1923`)
  scan JSONL files -> `SessionInfo { path, id, cwd, name?, created, modified,
  messageCount, firstMessage }`. No index, no DB.
- **Format** (`docs/session-format.md`): header line, then `{type,id,parentId,..}`
  entries; `message` entries hold the `AgentMessage` verbatim.
- **Resume / render**: `SessionManager.open(path)`; `buildSessionContext()` for
  the LLM; `renderInitialMessages()` (`interactive-mode.ts:4392`) pushes stored
  entries through the same components as live messages.
- **Reconnect** (RPC): no replay; `get_state` + `get_messages`, or
  `get_entries { since }` (`rpc-mode.ts:657`), then resubscribe.

Convergence: reopen = persisted items through the SAME renderer as live.

## 4. What to adopt, what not to copy

Target: `moneyball-desktop` links `moneyball-core` as the TUI does; commands in
via `#[tauri::command]`, events out via one `tauri::ipc::Channel<UiEvent>`.

### Adopt

1. **pi's seam, not codex's protocol.** Direct methods + one event stream. A
   Tauri command per Op and a forwarder thread `rx.recv() -> channel.send()` is
   the whole of `rpc-mode.ts:362`.
2. **A serializable, id-keyed UI event enum** (codex's item lifecycle):
   ```rust
   #[derive(Serialize)] #[serde(tag = "type", rename_all = "snake_case")]
   enum UiEvent {
     TurnStarted  { turn_id },
     MessageDelta { item_id, delta },              // disposable
     MessageDone  { item_id, text },               // authoritative
     ToolBegin    { call_id, name, args },
     ToolEnd      { call_id, ok, output, detail }, // detail = UI-facing half (ARCH 6b)
     ItemDone     { item: Item },                  // persist point, unchanged
     TurnEnded    { turn_id, status: completed|interrupted|failed, error?, ms, usage },
   }
   ```
   One terminal event with a status (codex `TurnStatus`) gives the webview
   reducer a single "turn over" path.
3. **Final event carries the full payload; deltas never persist** (already
   invariant 3). The forwarder may then coalesce deltas (16-33 ms) safely - the
   codex `requires_delivery` split without the machinery.
4. **One projection for live and reopen** (codex `ThreadHistoryBuilder`, pi
   `renderSessionEntries`): a core fn `&[Item] -> Vec<ViewItem>` that joins
   `ToolCall`+`ToolOutput` by `call_id` into one tool row with a status. The
   TUI cell rebuild should call it too, so the front-ends cannot drift.
5. **Snapshot-then-stream for reload** (codex running-thread resume, pi
   `get_state`+`get_messages`). A webview reload is our "reconnect". Live session
   state stays in Rust (`tauri::State`); `session_snapshot(id) -> { items,
   in_flight: Option<{item_id, text_so_far, open_tool_calls}>, status }`.
   Webview: subscribe, buffer, snapshot, apply buffer.
6. **Session list from headers** (pi `SessionInfo`): extend `SessionMeta` with
   first-user-text preview, modified time, item count. Directory scan, no index.
7. **Thin terminal events + immediate start return** (codex): `send_input`
   returns `turn_id` and a disposition (started | queued, as pi's `prompt` does);
   the transcript arrives only via events. One in-flight turn (invariant 2).
8. **TS bindings generated from Rust** (codex: `ts-rs` + schema export,
   `app-server-protocol/src/export.rs`). We already have schemars and
   `tests/schema_contract.rs`; pin `UiEvent`/`Item`/`SessionMeta` the same way.

### Do NOT copy

| Thing | Where | Why it exists | Why we skip |
|---|---|---|---|
| JSON-RPC envelope, `RequestId`, method-name registry, response-as-`serde_json::Value` | `rpc.rs`, `common.rs` macros | untyped wire between processes/languages | Tauri commands are already typed request/response. `in_process.rs` header admits the in-process path still pays the envelope only to "avoid a second execution contract" with stdio/ws - we have no first one |
| `initialize`/`initialized` handshake, capabilities, `optOutNotificationMethods`, `experimentalApi` | `v1.rs:57,69` | version/capability skew between separately shipped client and server | one binary, one version |
| Connection ids, per-thread subscriber sets, `thread/unsubscribe`, listener command queue | `thread_state.rs:406-766` | many clients on one thread | one window per session; a `Channel` handle is the subscription |
| `ServerRequest` round-trips (`item/*/requestApproval`, `item/tool/requestUserInput`, `item/tool/call`), `abort_pending_server_requests` | `common.rs:1899-1930` | approvals, sandbox escalation, client-hosted tools | read-only advisor: no approvals, no sandbox, tools run in core |
| Bounded queues, `WouldBlock`, `Lagged{skipped}`, overload error codes | `in_process.rs` "Backpressure" | untrusted/slow remote consumer | chat-cadence events to a local webview; coalescing deltas is enough |
| `ThreadStatus::NotLoaded` + load/unload/archive, cursor pagination, `TurnItemsView`, state DB, search | `thread.rs`, `thread_data.rs` | thousands of threads, huge transcripts, remote storage | tens of sessions; ARCH 6b says no DB, full history each request |
| Separate `ThreadItem` universe with ~17 variants and a 6k-line builder | `v2/item.rs`, `thread_history.rs` | legacy rollout formats + features we lack (exec, patches, MCP, collab, review, plan) | our `Item` has 4 variants; the projection is a `call_id` join |
| Transports, daemon, control socket, auth, websocket compression | `app-server-transport`, `app-server-daemon` | remote/IDE clients | none |
| pi RPC mode (JSONL framing, `id` correlation, stdout backpressure, extension UI records) | `modes/rpc/*` | child process, other languages, extensions | pi's own docs say use the SDK in-process |
| pi `message_update` full-partial snapshots on every delta | `types.ts:538` | free in-process in JS (shared reference) | across Tauri IPC it serializes the whole message per token |
| pi session tree (`parentId`, fork, branch summaries, labels, compaction entries) | `session-manager.ts` | branching/compaction features | linear append-only JSONL is binding (ARCH 6b) |
| p2 / p2-mac architecture (local HTTP server + WKWebView + owner tokens) | `apps/p2`, `apps/p2-mac` | multi-device, self-hosted server | explicitly experimental; reintroduces the process boundary and a loopback auth problem |

## 5. Open risks

1. **Docs forbid it as written.** AGENTS.md: "TUI is the management surface; no
   web UI"; ARCH 1: crate topology locked. Amend both first, and extend the
   arch_contract network-boundary and file-cap tests to the new crate.
2. **The seam in code is narrower than ARCH 6b says.** No `Op` enum exists in
   `crates/`; the TUI calls `agent::run_turn(.., cancel, tx: &Sender<Ev>)` itself
   and owns history, session append and the cancel flag (`moneyball-tui/src/
   {agent_turn.rs,app.rs}`). Without extracting a core "session driver" (history
   + `SessionLog` + worker + cancel; Ops in, events out) desktop duplicates that
   orchestration. This is the real work item; the Tauri glue is small.
3. **Tools live in the TUI crate.** `ToolExec` is implemented by
   `moneyball-tui/src/agent_tools.rs` (`SnapshotTools`). It, and the UI-facing
   `detail` half of tool results, must move to core before desktop can run a turn.
4. **`Ev` is not `Serialize` and deltas carry no id.** Fine for one linear TUI
   transcript; a webview that reloads mid-stream needs `item_id` on deltas and a
   Rust-side in-flight accumulator (4.5).
5. **Replay source.** ARCH 6b persists "final UI events for replay". If the TUI
   replays those while desktop projects from Items, they diverge. Both references
   project from items only; pick that and keep readers tolerant of old lines.
6. **Flows are TUI code.** `refresh_flow.rs` (its own `Ev`), `setup/`,
   `schedule_view.rs`, `diagnose_flow.rs`, `crm_flow.rs` etc. Desktop needs each as
   a core-driven op or ships chat-only - and must not advertise the rest.
7. **Threading.** Core is blocking threads + std mpsc; Tauri commands run on
   tokio. Keep core blocking: one forwarder thread per live session bridging
   `Receiver<Ev>` to `Channel::send`. Do not inherit codex's tokio host.
8. **Two writers on one session file.** TUI and desktop on the same workspace
   can append to one JSONL (`SessionLog` opens per append, no lock). codex
   serializes through one server; p2 has `runtime/session-lock.ts`. Minimum: an
   advisory lock on resume; no live cross-front-end viewing.
9. **IPC volume.** Batch deltas in the forwarder; large tool `detail` once on
   `ToolEnd`. **ASCII rule:** core strings stay ASCII; decide webview scope.
10. **Reference drift.** Both repos changed a lot since the 2026-07-16 research
    (codex: paginated rollout, `ThreadItemPayload` wrapper, in-process host; pi:
    web-ui removed, durable/chord/p2 added). Cite the pinned commits; ignore p2.
