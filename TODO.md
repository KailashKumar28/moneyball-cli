# TODO - moneyball project backlog

Single source of truth for open work. Keep it honest: check items off in
the same commit that ships them, add new items when scope is agreed, and
delete items that stop mattering. Order within a section = priority.
(Conventions: `[ ]` open, `[x]` done - move done items to the log at the
bottom after a release-sized batch.)

## Now (next slices, in order)

- [ ] **LeadZump date filter**: today the preset pulls the whole ticket
      book (`condition: null`, no date window) and hits MAX_PAGES=500 on
      every connect. Evidence + DSL details in docs/CRM_CONNECTORS.md.
      Verify the `operator` for a date comparison against the live
      endpoint, then update the preset's body template; while there,
      teach `crm/fetch.rs` to honor `totalElements` so the loop terminates
      cleanly without the backstop.
- [ ] **Validate LeadSquared for real**: point a `crm.toml` at a live
      LeadSquared account (AccessKey/SecretKey), confirm the preset field
      paths (`mx_*` attribution, ProspectStage) and paging behavior.
- [ ] **LeadZump via existing endpoint**: build `crm.toml` from the old
      fin_campaign_analysis pipeline's JSON endpoint; keep the
      "Stattic Ad" typo untouched (join rule - see AGENTS.md Don'ts).
      (Superseded by the LeadZump date-filter item above once that lands.)
- [ ] **/diagnose <product>**: the 5 diagnostic checks over a snapshot,
      one summary cell + per-check detail. Headless first.

## Next

- [ ] **Audit package G remainder** (structure P1/P2s from the 2026-07
      architecture audit): move `connect_flow.rs` orchestration into
      core behind an ask() seam; dedupe the three brief formatters;
      split `submit()` and bring commands.rs (~800) / setup/mod.rs
      (~980) / llm.rs back under the 400-line cap; snap_dir() at all
      call sites; module-doc drift (tools.rs done, check others);
      /brief double-context (short user item + let the loop call the
      brief tool); remaining P2 hygiene (ASCII strings, busy indicator,
      IST vs Local snapshot dating, query_params support, usage/cost
      parsing, atomic config writes, render examples for new cells).

- [ ] **/ledger**: wire the prediction ledger view (stub today). Append
      predictions to `ledger.jsonl`, show hit-rate over time.
- [ ] **Scheduled CRM fetch**: document cron/launchd recipe for
      `moneyball crm fetch` next to the existing `moneyball fetch` one
      (both write into the same day's snapshot dir).
- [ ] **Surface CRM in the TUI**: `/crm` command showing connection
      status (spec present? last crm.json date? join rate) with a
      pointer to the headless commands; longer term, a TUI-native
      `/crm connect` view wrapping `crm::connect`.
- [ ] **Setup wizard: CRM step**: offer `crm init` + contract pointer at
      the end of /setup instead of leaving CRM discovery to docs.
- [ ] **MB_AGENT output coverage**: machine-readable JSON for `brief`,
      `crm check`, `crm fetch` (exists for some sub-commands only).
- [ ] **Auto-compaction**: real history compaction (LLM summary of old
      turns) once sessions regularly outgrow the context window - /clear
      is the manual escape hatch shipped 2026-07-17; codex's /compact is
      the reference.

## User-side / verification (not code)

- [ ] Confirm wheel-scroll works in the real terminal (fix shipped;
      needs a human hand on a real mouse).
- [ ] Configure an LLM provider in the `fin_campaign_analysis` workspace
      (/setup step 4) - brief there currently says "no LLM configured".
- [ ] LeadSquared credentials needed for the live validation item above.

## Ongoing hygiene (never a ticket, always the rule)

- Clippy stays at zero; size caps per ARCHITECTURE §3; every slice ends
  with `cargo install` (gate #7); E2E-reproduce bugs before fixing them.

## Done log

- [x] 2026-07-18 LeadZump connected for real (user's live token): the
      blocker was the contract treating ad_id as required per ticket
      while every real CRM holds organic/direct leads - transform now
      drops no-ad-id records and counts them (matches the production
      pipeline, which keys its export by ad id; "Stattic Ad"
      placeholders are non-empty and kept per the join rule). Reports
      say "141 ad-attributed ticket(s) (154 organic/direct dropped)".
      preset re-runs keep stored credentials on empty input. E2E on
      moneyball-test: connect -> PASS -> "Fresh" mapped keep-as-is ->
      crm.json written -> brief shows real l7d/q7d.

- [x] 2026-07-17 Audit package G core: deleted the dead async LLM
      client stack (~500 lines: Client, complete/complete_with_tools,
      tool body builders, parse_completion parsers, Completion/
      ToolResult/tools_payload + their tests) - llm.rs 1442 -> 850ish;
      run_turn now takes a StreamFn seam (pi pattern) with four
      hermetic loop tests (tool round + ItemDone ordering, length-stop
      fails calls without executing, cancel -> marker + TurnAborted,
      error -> Failed with no items); SessionLog got a
      MONEYBALL_SESSIONS_DIR seam + on-disk create/append/open round-
      trip test; the stages+snapshot+check prelude that was copy-pasted
      in fetch/connect/main is now crm::check_with_workspace. E2E:
      headless brief + crm check on live workspace unchanged (98% join).

- [x] 2026-07-17 Agent-core P1 pair: (1) items now persist AS THEY
      COMPLETE via Ev::ItemDone (codex rollout rule) - terminal events
      carry no payload and the base_len/split_off machinery is gone
      entirely, so the shifted-window bug class cannot recur; a crash
      mid-turn loses at most the item in flight (E2E: session JSONL
      shows user/tool_call/tool_output/assistant appended during the
      turn). (2) Context-overflow escape hatch: /clear (alias /new)
      starts a fresh history + session file (old one stays resumable),
      and context-window errors get an actionable /clear hint in the
      failure cell. E2E: /clear -> turn -> /quit -> --continue resumes
      the fresh session correctly. ARCHITECTURE 6b seam updated.

- [x] 2026-07-17 Audit package F (truth + TUI P1s): /diagnose and
      /ledger stubs removed from COMMANDS, /help, and the agent prompt
      (dead tools.rs registry deleted with them); /keychain registered;
      /help now generated from COMMANDS so the three command surfaces
      cannot drift; AGENT_SYSTEM_PROMPT tells the truth (two tools,
      never answer from memory) instead of claiming snapshot-in-context;
      funnel tool description matches its real adset-table output and
      requires product; agent ToolCall cells finalize on ToolEnd with
      real name + measured duration (0 renders as no duration, never
      "(0ms)"); scroll offset clamped to content (Home no longer parks
      the view thousands of lines past the top); logo actually spells
      MONEYBALL; CRM contract doc no longer instructs partners to create
      the ads-free snapshot dirs the code refuses. E2E vs MiniMax:
      tool cell goes Running -> Done with duration, /help + logo render.

- [x] 2026-07-17 Audit package E (turn robustness P1s): SSE error events
      and 200-status error bodies now fail the turn loudly (per-wire
      classify + terminal-event tracking; empty terminal-less streams
      are errors, never Ok("")); stop_reason/finish_reason observed -
      length stops fail ALL tool calls back to the model (pi contract)
      and cut-off plain answers get a visible note; agent turns on the
      Messages wire raised to 8192 max_tokens; stream_turn restored the
      LlmAuth contract via a shared status_error; TUI panic hook
      restores the terminal (codex pattern) and restore() is best-effort
      per step; a dead agent worker now releases the turn with a visible
      cell instead of bricking Esc. Live E2E vs MiniMax passed.

- [x] 2026-07-17 Audit package D (all five P0s from the full architecture
      audit): heal_history now runs before base_len capture (Esc during a
      tool no longer corrupts sessions with per-turn duplicates); snapshot
      dir names validated as dates in snap_for + snapshot::load (stray
      dirs error cleanly instead of panicking brief/funnel - E2E verified);
      empty CRM exports refuse to overwrite crm.json (E2E: header-only CSV
      import leaves good data untouched); pasted-curl query-param
      credentials secretized before disk/LLM (per-param prompt, E2E via
      tmux); secrets tests hermetic via MONEYBALL_AUTH_PATH seam + atomic
      auth.json save (tmp+rename, pid-unique).

- [x] 2026-07-17 CRM keys-only connect shipped: preset catalog
      (LeadZump proven, LeadSquared docs-based), crm connect = pick CRM
      -> paste keys (with where-to-find help) -> live test pull gates
      the save; custom path takes a pasted curl command (POST bodies
      work), auto-secretizes headers, LLM drafts the map; observed
      unknown stage names get mapped interactively and written back to
      crm.toml; paging semantics validated at parse time so the LLM
      retry loop self-corrects. rustyline editing on all prompts.

- [x] 2026-07-16 Package C shipped: char-aware Left/Right (multibyte
      input no longer panics - tmux-verified with "cafe"+accent), single
      Ctrl+C clears input + arms, second within 2s quits, /setup Esc
      clears input -> steps back -> exits to chat (hint now true),
      paste is cursor-aware with control chars stripped.

- [x] 2026-07-16 Package B shipped: bracketed paste (multi-line paste =
      one editable insert, never auto-submits), unknown /commands
      rejected locally (no paid LLM on typos), crm fetch/import refuse
      to create ads-free "latest" snapshots, /brief + agent tool + state
      block all warn when crm.json is absent (model now blames setup,
      not fake zeros), CRM request errors scrubbed via without_url(),
      remaining byte-slice truncations fixed via crm::source::truncate_chars.

- [x] 2026-07-16 Package A shipped: agent brain live. History = wire
      format (agent::Item), pi tool loop with brief+funnel tools the
      model invokes itself, append-only JSONL sessions with real replay
      (-c resume verified: follow-up questions resolve across restarts),
      AtomicBool cancellation, latency as status metadata. E2E vs
      MiniMax: "what was my last question" answers correctly.

- [x] 2026-07-16 /funnel wired in the TUI: table tool cell + streaming
      LLM per-entity SCALE/KILL/WAIT read; bad args list the configured
      products. Headless funnel already at exact mb.py parity.

- [x] 2026-07-16 funnel headless: `moneyball funnel <product> --by
      campaign|adset|ad --window N` - exact numeric parity vs
      `pipeline/mb.py funnel` on live data (incl. kill table, learning
      status, CRM join). Milestone semantics unified in crm::milestones.

- [x] 2026-07-16 CRM phase 4 (headless): `moneyball crm connect` - probe
      endpoint, LLM drafts crm.toml once (parse-retry loop against a
      strict deny_unknown_fields schema), dry-run over the live sample,
      save on approval. E2E: MiniMax drafted a working spec from a mock
      CRM; saved spec ran via `crm fetch` deterministically.
- [x] 2026-07-16 TUI freeze on /fetch and /brief self-heal: Meta pull
      moved to a worker thread (StreamEvent::FetchDone), tmux-verified.
- [x] 2026-07-16 CRM phase 3: CSV import through the same map +
      validation pipeline; `[request]` optional in crm.toml.
- [x] 2026-07-16 CRM phase 2: declarative `crm.toml` executor -
      `crm init/fetch/secret`, paging, stage_map, validator-gated write.
- [x] 2026-07-16 CRM phase 1: `docs/CRM_CONTRACT.md` + `crm contract` +
      `crm check` conformance validator (ad_id join-rate diagnostic).
- [x] 2026-07-16 lib.rs split to table of contents; clippy to zero;
      llm.rs header dedupe (ARCHITECTURE §8 cleared).
- [x] 2026-07-15 auth.json (0600) replaces OS keychain; `.moneyball/`
      dot-dir convention + legacy migration.
- [x] 2026-07-15 streaming LLM responses (worker + mpsc drain), command
      palette, caret-first placeholder, wheel scroll, /fetch self-heal
      for /brief.
