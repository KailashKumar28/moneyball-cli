# TODO - moneyball project backlog

Single source of truth for open work. Keep it honest: check items off in
the same commit that ships them, add new items when scope is agreed, and
delete items that stop mattering. Order within a section = priority.
(Conventions: `[ ]` open, `[x]` done - move done items to the log at the
bottom after a release-sized batch.)

## Now (next slices, in order)

- [ ] **Package A - agent brain** (design: ARCHITECTURE.md §6b):
      A1 `core/agent/protocol.rs`: Item/Op/Ev enums (history = wire format).
      A2 llm.rs body builders take `&[Item]` + tool defs; parse tool calls.
      A3 agent worker thread: full-history turn loop, tool registry wired
         (brief + funnel first), pi truncation caps, AtomicBool cancel +
         <turn_aborted> marker, invariants (outputs healed, one turn).
      A4 sessions -> append-only JSONL of Items (+ final Evs); resume
         replays into history AND cells; delete the destructive
         load_session_into/save_current_session pair.
      A5 TUI: replace StreamEvent plumbing with Op/Ev seam; ToolBegin/End
         cells keyed by call_id; latency/provider as cell metadata.
- [ ] **Package B - money/data safety**: bracketed paste; reject unknown
      /commands locally; real Esc cancellation (A3 gives the flag); crm
      fetch must not create ads-free "latest" snapshots; warn when crm.json
      absent (brief) instead of narrating fake zeros; scrub URLs/secrets
      from CRM error strings; fix byte-slice truncations (main.rs preview,
      crm/fetch preview, llm.rs truncate).
- [ ] **Package C - input safety + exits**: char-aware cursor moves +
      completion slices; double-Ctrl+C to quit; /setup Esc actually goes
      back/exits; paste into chat respects cursor + strips control chars.
- [ ] **UX polish batch** (from UX critique): spinner + elapsed + esc hint
      while working; context bar middle-truncation (snapshot/status always
      visible); single command registry drives palette + /help + footer
      (add /help /keychain /exit, drop stubs); fix garbled logo; funnel
      table drops id column first on narrow widths; Up = input history.

- [ ] **Validate LeadSquared for real**: point a `crm.toml` at a live
      LeadSquared account (AccessKey/SecretKey), confirm the preset field
      paths (`mx_*` attribution, ProspectStage) and paging behavior.
- [ ] **LeadZump via existing endpoint**: build `crm.toml` from the old
      fin_campaign_analysis pipeline's JSON endpoint; keep the
      "Stattic Ad" typo untouched (join rule - see AGENTS.md Don'ts).
- [ ] **/diagnose <product>**: the 5 diagnostic checks over a snapshot,
      one summary cell + per-check detail. Headless first.

## Next

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
