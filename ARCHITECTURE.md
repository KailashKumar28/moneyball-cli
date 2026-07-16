# ARCHITECTURE.md - structure & style contract

Binding for every contributor, human or agent (Claude Code, pi, codex).
Read this before writing code. `AGENTS.md` covers process and domain rules;
this file covers how code is shaped. When they conflict, AGENTS.md wins.

Reference architectures: **openai/codex** (`codex-rs`: layered crates, TUI
split into one-concern files like `app.rs`, `app_event.rs`, `chatwidget.rs`,
`bottom_pane/`) and **badlogic/pi-mono** (small core, strict package layering
`ai -> agent -> coding-agent`, minimalism as a feature). We deliberately do
not use hermes-agent as a reference (see AGENTS.md Don'ts).

## 1. Crate topology (locked)

```
moneyball-core   headless logic: snapshot, brief, fetch, llm, config, secrets
moneyball-tui    ratatui front-end; consumes core as a library
moneyball        thin clap binary; dispatch only, no logic
```

Dependency rule: `moneyball -> moneyball-tui -> moneyball-core`, never the
reverse. Core has no ratatui/terminal dependency, ever. The same code path
is exercised headless (sub-commands) and through the TUI.

Network boundary: the read/analysis path (brief, funnel, advisor math)
never touches the network. Network lives in exactly four core modules:
`meta.rs` (account discovery), `fetch.rs` (explicit snapshot pull),
`llm.rs` (model calls), `crm/fetch.rs` (explicit CRM pull driven by the
declarative crm.toml spec). Nothing ever writes to Meta or the CRM.

## 2. I/O contract

- Reads: snapshots at `<workspace>/.moneyball/history/snap/<date>/`.
- Writes: `.moneyball/{config.json,crm.toml,history/,state/,runs/}` in
  the workspace; `~/.moneyball/auth.json` (0600) for secrets - codex-style
  dotfile, not the OS keychain (keychain ACLs break for locally built
  binaries). Secrets never appear in config.json, crm.toml, or logs -
  crm.toml references them as `secret:<name>` / `env:<VAR>`.
- CRM data enters only as contract-conformant `crm.json`
  (docs/CRM_CONTRACT.md); `crm::check` gates every write of it.
- `MB_AGENT=1` -> machine-readable output for sub-commands.

## 3. Module rules - the anti-verbosity contract

- **One concern per file.** A view, a widget, a command dispatcher, a
  protocol - each gets its own module (codex-tui's shape). God-files are
  defects.
- **Soft caps: ~400 lines per file, ~40 per function.** Crossing one is a
  signal to split, not an excuse for a waiver. `moneyball-tui/src/lib.rs`
  at 3,200+ lines is the standing counterexample - see §8.
- **lib.rs is a table of contents**: module declarations, re-exports, and
  root-owned types only. No business logic.
- New TUI surface -> new file (e.g. `setup/`, `palette.rs`, `commands.rs`),
  registered in lib.rs. Never appended to an unrelated module.

## 4. Style rules

- **Every line earns its keep** (pi's rule). No dead code, no speculative
  abstractions, no forwarding-only wrappers. Delete code the moment it's
  orphaned; `#[allow(dead_code)]` requires a written reason.
- **Say it once.** The same logic in two places is a smell; in three, a
  defect (see §8: `llm.rs` header building). Extract one helper at the
  lowest layer that needs it.
- **Errors**: `thiserror` enums in core; `anyhow` only at the binary edge.
  No `unwrap()`/`expect()` in library code except provably-infallible
  cases with a comment saying why.
- **Comments explain *why*, not *what*** - terse, high-signal. Doc-comments
  on public items. A comment restating the line below it gets deleted.
- **Naming**: one concept, one name, everywhere (snapshot, workspace,
  product, provider). Never introduce a synonym for an existing concept.
- **User-facing strings** are sentences with a next action - never raw
  internal errors or placeholder tokens (`<any>`) leaking to the screen.

## 5. TUI patterns (codex-derived)

- Transcript content = `ChatCell` trait objects (`chat.rs`), one cell type
  per content kind (codex's `HistoryCell`). New content kinds are new
  cells, not string formatting inside the render loop.
- Rendering is pure: `render(frame, &App)` reads state, never mutates it.
  Mutation lives in key/event handlers and the stream drain.
- Long work never blocks the event loop: worker thread + `mpsc`, drained
  on the 100ms tick (the LLM streaming path is the template).
- Failed tool cells are never the last word: follow with a plain-language
  assistant cell saying what's needed. One error surface - no duplicating
  the same failure in the status line.
- Every view renders via `TestBackend` (`render_to_string`); that is both
  the test surface and the review surface. New view -> render example or
  snapshot test, and actually read the output before shipping.

## 6. Provider/LLM patterns

- All wire-protocol differences live behind `WireApi` in exactly two
  places: request building and response parsing. Callers never branch on
  provider identity.
- New provider = a preset in `provider.rs` + a wizard list entry. If it
  needs code anywhere else, the abstraction is broken - fix the
  abstraction instead.
- Base URLs are verified against the live endpoint before shipping (a 404
  from a preset is a preset bug, not a user error).

## 6b. Agent core (binding; researched from codex-rs + pi-mono source, 2026-07-16)

The conversational layer follows openai/codex (codex-rs) and badlogic/pi-mono,
which independently converge on the same primitives. Deviating from these
requires updating this section first.

- **History is the wire format.** One `Item` enum is BOTH the in-memory
  transcript and the persisted format (codex's `ResponseItem` trick):
  `User{text} | Assistant{text} | ToolCall{call_id,name,args} |
  ToolOutput{call_id,output,is_error}`. No separate chat-message model.
  Every request sends the FULL history (no incremental diffing, no DB).
- **The loop** (pi's shape): send history -> stream response -> collect tool
  calls -> execute -> append `ToolOutput`s -> repeat until a response has no
  tool calls. No iteration cap. A tool failure is a `ToolOutput{is_error}`
  message fed back to the model - never an exception, never a dead turn.
- **Invariants** (codex): (1) every `ToolCall` has a `ToolOutput` before the
  next request - synthesize "aborted" outputs at prompt build; (2) exactly
  one in-flight turn per session (new input queues or replaces); (3) the
  final assistant event carries the complete text, so deltas never persist.
- **Cancellation**: UI thread sets a shared `Arc<AtomicBool>`; the worker
  checks it between SSE events (blocking reqwest iterator returns between
  events - same latency as codex's `or_cancel`) and drops the stream
  (TCP-level cancel). After interrupt, append a user-role
  `<turn_aborted>...` marker item; heal dangling calls lazily.
- **Sessions**: append-only JSONL - header line, then one `Item` per line
  (plus final UI events for replay). Resume = read lines, rebuild history
  from Items and cells from final events. Never rewrite the file.
- **Tool output truncation** (pi): every tool result is capped (~2000 lines
  / 50KB, head-keep for reads, tail-keep for command output) with a
  continuation hint in the text.
- **Tool results split model-facing `content` from UI-facing detail** so the
  table the user sees is not necessarily the tokens the model pays for.
- **Minimalism is load-bearing** (pi): short system prompt (<1k tokens), no
  RAG/embeddings, no sub-agents, no planner state. Models are RL-trained to
  understand agent loops; the loop + files + a few sharp tools is the
  product. Escape hatches go through existing commands, not new machinery.
- **TUI<->core seam**: `Op` in (UserInput/Interrupt/Shutdown), `Ev` out
  (TurnStarted, AssistantDelta/Done, ToolBegin/End keyed by call_id,
  TurnAborted, TurnComplete, Error) over std mpsc. The TUI creates a cell
  on ToolBegin and finalizes it on ToolEnd; it never sees wire formats.

## 7. Enforcement gates (definition of done for any change)

1. `cargo fmt --all` - default rustfmt, no config debates.
2. `cargo clippy --workspace --all-targets` - zero NEW warnings; drive the
   existing count down, never up.
3. `cargo test --workspace` green; new behavior ships with a hermetic test
   (no network, no real accounts).
4. Affected views re-rendered via TestBackend and read.
5. Size caps (§3) respected for touched code.
6. `cargo install --path crates/moneyball --locked` - the user runs the
   installed binary; a fix that isn't installed isn't shipped.

## 8. Standing debt (chip away; never add to it)

- [x] lib.rs split complete (2026-07-16): lib.rs is a 96-line table of
      contents; app.rs / event.rs / render.rs / commands.rs / palette (in
      render) / setup/{mod,render_steps}.rs all within or near the soft cap.
- [x] `llm.rs`: `request_headers()` is the single source of truth for
      auth/header assembly across all three call paths.
- [x] Clippy at zero (2026-07-16). Hold there (gate #2).
- [x] Dead tui code removed (render_brief, render_input_bar, truncate,
      comma) during the chat-view rework.
