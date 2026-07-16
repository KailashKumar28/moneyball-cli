# TODO - moneyball project backlog

Single source of truth for open work. Keep it honest: check items off in
the same commit that ships them, add new items when scope is agreed, and
delete items that stop mattering. Order within a section = priority.
(Conventions: `[ ]` open, `[x]` done - move done items to the log at the
bottom after a release-sized batch.)

## Now (next slices, in order)

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
