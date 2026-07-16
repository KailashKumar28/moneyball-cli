# TODO - moneyball project backlog

Single source of truth for open work. Keep it honest: check items off in
the same commit that ships them, add new items when scope is agreed, and
delete items that stop mattering. Order within a section = priority.
(Conventions: `[ ]` open, `[x]` done - move done items to the log at the
bottom after a release-sized batch.)

## Now (next slices, in order)

- [ ] **/crm connect wizard (CRM phase 4)**: TUI flow - user pastes base
      URL + auth + one sample response; LLM drafts `crm.toml` ONCE; show
      the draft; `crm check` must PASS before saving. LLM never in the
      recurring data path. Headless parts exist (`crm init/fetch/check`).
- [ ] **Validate LeadSquared for real**: point a `crm.toml` at a live
      LeadSquared account (AccessKey/SecretKey), confirm the preset field
      paths (`mx_*` attribution, ProspectStage) and paging behavior.
- [ ] **LeadZump via existing endpoint**: build `crm.toml` from the old
      fin_campaign_analysis pipeline's JSON endpoint; keep the
      "Stattic Ad" typo untouched (join rule - see AGENTS.md Don'ts).
- [ ] **/funnel <product>**: per-entity funnel (campaign -> adset -> ad:
      spend, m/l/q/v/b). Headless sub-command first, then TUI. Meta-side
      numbers work today; CRM stages now available via crm.json.
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
      pointer to the headless commands.
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
