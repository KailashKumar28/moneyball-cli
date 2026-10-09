# TODO - moneyball project backlog

Single source of truth for open work. Keep it honest: check items off in
the same commit that ships them, add new items when scope is agreed, and
delete items that stop mattering. Order within a section = priority.
(Conventions: `[ ]` open, `[x]` done - move done items to the log at the
bottom after a release-sized batch.)

## Now (next slices, in order)

- [ ] **Daily refresh and data health** (picked 2026-10-08 by a three-engineer
      review panel; design: plans/prototype/board.html sections R and S;
      contract: plans/daily-refresh-build-prompt.md; the task plan and its
      verified evidence live in plans/prototype/build.js, which the board
      reads). Open decisions: launchd write boundary, CRM-fails policy,
      LLM after refresh.
  - [x] Verdict gate (2026-10-09): no kill / 0-CRM verdict from missing CRM
        data, in funnel rows, the table every surface prints, and the
        report's targeting chips. The 27 Sep report had 11 false Kill chips.
  - [x] CRM health (2026-10-09): one type and one sentence for the CRM's
        state (not connected / connected but not fetched / fresh); the
        headless brief now warns, and crm status names the snapshot crm
        fetch really writes into.
  - [x] Account isolation (2026-10-09): one ad account failing marks its
        product "no data" with Meta's reason; the others still land.
  - [x] moneyball refresh (2026-10-09): Meta, then CRM anchored to the
        same snapshot, then the report, under one lock, with a run record
        (moneyball.run/1) written whatever happens; exit 0/2/1; MB_AGENT
        JSON. Live: the 08 Oct report written, CRM degraded on the expired
        LeadZump token with its exact fix. Typed MetaAuth / CrmAuth errors.
  - [x] /refresh in the TUI (2026-10-09): a live step table drawn to the
        board's frames, the verdict, the fix, the day's numbers and the
        report after it; on its own channel so chat keeps working; Esc
        stops after the step in flight; ctrl+o opens the report. /fetch is
        an unlisted alias and /brief self-heals through /refresh.
  - [x] Data health (2026-10-09): the context bar says the data day, the
        CRM day and one state (fresh / stale / CRM missing / N of M
        products / refresh failed) instead of "ready"; the landing says
        what is wrong and the one command that fixes it, or yesterday's
        numbers and report when all is well.
  - [x] Report provenance (2026-10-09): report.json says when Meta and CRM
        were pulled and where CRM stood; the client report hides every CRM
        figure when there is none (never zeros), says when its data was
        pulled, and compares only within 7 days.
  - [x] /status and moneyball status (2026-10-09): the last 14 data days,
        missed days as rows, how Meta and the CRM last answered (frame R6).
  - [x] /schedule and moneyball schedule (2026-10-09): the morning refresh as
        a launchd agent, confirmed in-app before anything is written (R4);
        fired live, recorded trigger schedule, removed by /schedule off.
  - [x] /quit mid-refresh asks once (2026-10-09); a command that cannot run
        now answers in the transcript, never leaves its prompt bare.
  - [x] Funnel view (2026-10-09): people get frame S4's table (name, spend,
        meta, cpl, crm, qual, rs_per_q, one verdict) in ~85 columns; the
        model keeps the full table with ids and kill flags.
  - [x] /setup meta (2026-10-09): the Meta token fix that works today - the
        wizard opens at the token paste, a new token replaces the saved one
        only if it reads every configured account, products stay. Before,
        /setup jumped to the model step and the token could not be reached.
  - [x] Per-client CRM keys (2026-10-09, credentials v2 part 1): auth.json
        keeps CRM keys under each workspace, so two clients on one CRM never
        overwrite each other; the legacy global key still reads as a
        fallback. Secret prompts (crm secret, crm connect) show * only.
  - [x] One status of every source (2026-10-09, integrations A2 part 1):
        /connect and moneyball connect show ads, leads and the advisor as
        connected / refused / missing with each fix (MB_AGENT=1: JSON);
        /connect replaces /keychain (kept as an unlisted alias); moneyball
        connect crm runs the guided CRM flow.
  - [x] CRM connect inside the app (2026-10-09, integrations A2 part 2):
        the flow moved into core behind a Prompter; /connect crm runs it on
        a worker with the composer answering (secrets as *), moneyball
        connect crm in the shell. Every "connect a CRM" fix points there.
  - [x] Review follow-ups done (2026-10-09): /connect and /status date a
        source's last answer with one helper (status::last_answer); a
        reused pull without its file still reports, undated.
  - [x] Custom model provider protocol (2026-10-09): its URL says it - a
        bare base is Chat Completions (what every OpenAI-compatible server
        speaks; it was Responses, which most do not), a full endpoint
        ending /responses or /messages picks that wire. Anthropic's model
        list moves to claude-opus-5-5 / claude-sonnet-5 / claude-haiku-4-5.
  - [x] No shared CRM key fallback (2026-10-09): a pull reads only the
        workspace's own key; the legacy shared one is copied once into a
        workspace that has pulled CRM data (it used that key), and never
        reaches a new client. E2E on scratch copies: the new client reads
        "leadzump_token not stored", the old one adopts it.
  - [x] Pasted curl credentials (2026-10-09): -u becomes a Basic header,
        a JSON body's credential fields become {secret:<name>} refs (asked,
        like query params) filled JSON-escaped at request time; a name used
        twice (query and body both say apiKey) gets its own secret. E2E
        against a mock CRM: header, query and body each arrived filled.
  - [x] Meta calls share one GET (2026-10-09): a rate limit (4/17/32/613/
        80000s), a 5xx or is_transient waits 10s and goes once more, unless
        Meta's usage header says the block lasts minutes - then the error
        says how many; every paged call (insights, leads, the ad account
        list, which read one page of 200) follows paging.next.
  - [ ] Token expiry for /status ("Meta valid to ...") waits on the
        connect work.
- [ ] **Daily creative report** (plan: docs/CLOUD_PLAN.md, agreed
      2026-08-07 - report.json is the product, HTML is a renderer;
      reimplements pipeline/creative_report.py, never imports it):
  - [x] A0 - schema module: serde structs + schemars -> docs/schemas/,
        schema_contract test, fixture snapshot with creatives.json
        (2026-08-07).
  - [x] A1 - `/fetch` writes creatives.json (envelope, image_hash /
        video ids; asset refs filled by A2); fetch/ dir module keeps the
        network boundary. Docs updated; ~/.moneyball/reports/ renamed to
        bug-reports/ with migration (2026-08-07).
  - [x] A2 - content-addressed asset cache
        history/assets/creatives/<hh>/<sha256>.<ext>; full-res via
        adimages hash resolution + nested thumbnail params; refs reused
        from the prior snapshot, failures degrade to placeholder
        (2026-08-07).
  - [x] B1 - report.rs: snapshot -> report.json (grouping precedence,
        7-stage funnel, unattributed bucket, brief.rs Window reuse) +
        CLI text summary; golden-file parity vs the Python generator.
        Shipped 2026-08-07: per-product parity vs python HTML - M-leads
        exact, spend/impr within 0.6% (Meta restatement between pulls).
  - [x] B2 - HTML renderer, pure fn over report.json + assets only ->
        .moneyball/reports/<date>/creative-report.{json,html}; board
        layout, base64 images downscaled to card width (2026-08-07).
  - [x] C - TUI /report: worker-thread generate, summary + browser
        path as the tool cell (2026-08-07). FEATURE COMPLETE.
  - [x] Hedges (2026-08-07): fetch_snapshot takes token param;
        arch_contract denies HOME/cwd reads outside config.rs
        (config::home_dir is the seam). WorkspaceConfig::from_parts
        dropped consciously - all fields are pub, a literal suffices.
- [ ] **CRM lights up** (top UX debt, agreed 2026-08-08: every surface
      has honestly warned "no CRM data" for three weeks - the funnel
      columns are the product's point):
  - [x] LeadZump date filter - CLOSED as verified-impossible
        (2026-08-09, fresh token): GREATER_THAN_EQUAL exists in the DSL
        but createdAt is not comparable (HTTP 500 on every value
        shape). Client-side cutover + totalElements termination is the
        final design. Live: 523 ad-attributed tickets over 12 pages of
        a 154,927-ticket book; crm.json PASS.
  - [x] `/crm` status (TUI) + `moneyball crm status` (2026-08-08):
        spec/secret/newest-crm.json with delivery range, the
        stale-vs-missing call-out, and the one next command. Bonus from
        the live 401: auth failures now say "token expired + the fix"
        instead of "response is not JSON" (status checked before parse).
- [ ] **Advisor numeric reliability**:
  - [x] pre-computed cpl ranking in funnel output (TUI cell, agent
        tool, headless CLI); bug mb-20260808T143646Z-rrrr resolved -
        live re-test narrated the order correctly (2026-08-08).
  - [ ] model policy locally (cloud-plan rule): strong model for
        analysis turns, cheap only for mechanical jobs. Also observed:
        same data produced WAIT one run and KILL the next on minimax -
        recommendation stability is the remaining gap, and it is a
        model-quality issue, not a math one.
- [x] **Suggested commands always run when typed** (2026-08-08):
      product::resolve_product (exact > case-insensitive > unique
      substring, ambiguous errors with candidates) wired into TUI
      /funnel + headless funnel; app_state_block pins exact product
      names for the model. E2E: "Namma Mane" and "cityville" resolve,
      "Fincity" errors with the candidate list.
- [x] **Session-id entropy + no-clobber create** (2026-08-08): one
      xorshift state across the suffix + counter-mixed seed;
      File::create_new with retries. E2E: three same-second launches
      -> pj91/g3xq/v5gw.
- [ ] **Validate LeadSquared for real**: point a `crm.toml` at a live
      LeadSquared account (AccessKey/SecretKey), confirm the preset field
      paths (`mx_*` attribution, ProspectStage) and paging behavior.
- [ ] **LeadZump via existing endpoint**: build `crm.toml` from the old
      fin_campaign_analysis pipeline's JSON endpoint; keep the
      "Stattic Ad" typo untouched (join rule - see AGENTS.md Don'ts).
      (Superseded by the LeadZump date-filter item above once that lands.)
- [x] **Lead segmentation (the Diff breakdown)** shipped 2026-08-10:
      /fetch captures per-lead Meta records (leads.json, 0600 raw PII -
      user decision: raw ok locally, revisit at Postgres); crm map
      gains optional lead_id/phone/email + organic contacts file; the
      report classifies captured/re-inquiry/duplicate/invalid/
      uncaptured per card (python precedence, phone rules ported) and
      the table diff column expands (+) into re-inq/dup/other. Live:
      57 submissions -> 52 captured, 2 re-inq, 3 uncaptured. Known
      limit: CRM index = pulled window book, not lifetime.
- [x] **Report v2 - "Fincity Portfolio / Daily Performance"**
      (2026-08-10, designed by 3 specialist agents - marketing
      strategist, client-reporting, UI): targeting section per product
      (window table + 7d-basis verdict chips: kill/0-CRM/learning/
      weak-L>Q/scale, exact rules in report/targeting.rs), adset specs
      captured by /fetch (geo/age/learning - also lights the dormant
      Live-learning card status), exec brief (deterministic G1-G6),
      missing-leads banner, scorecard deltas vs prior report,
      reconciliation line that must sum, products-at-a-glance,
      client-facing labels (Meta leads/CRM leads/gap/missing),
      dead-tail collapse, mobile sticky columns, brand config +
      WhatsApp-forwardable filename (Fincity-Daily-09-Aug-2026.html).
      Deferred to weekly: fatigue + overlap verdicts (need reach /
      geo-circle math), budget reallocation modeling, cross-read B.
- [ ] **/diagnose <product>**: the 5 diagnostic checks over a snapshot,
      one summary cell + per-check detail. Headless first.

## Next

- [x] **Multi-day prior comparison** (2026-08-13): load_prior scans
      every `creative-report*.json` per day dir; the window-length
      check picks the comparable one - weekly/month runs now get G2
      and scorecard deltas.
- [x] **Segmentation parity check** (2026-08-13): python seeds the
      seen-sets unconditionally too (invalid leads included) - Rust
      already matched, no change. Real find instead: Meta lead
      created_time is UTC (+0000) while the window is IST days; ~10%
      of live leads (18:30-24:00 UTC) bucketed to the wrong day.
      Segmentation now converts to IST before dating, like python.

- [x] **Leads pull cost** (2026-10-09): measured - leads were 24s of a
      39s pull, insights 8-14s, one GET at a time. fetch::in_parallel runs
      the per-ad leads and the per-account insights 6 at a time, results in
      item order: the real refresh's Meta step went 64-75s -> 18s with
      identical ads_daily and leads.json. Lifetime leads kept (a duplicate
      needs the person's earlier submissions), no time_created window.

- [ ] **Audit package G remainder** (structure P1/P2s from the 2026-07
      architecture audit): move `connect_flow.rs` orchestration into
      core behind an ask() seam; dedupe the three brief formatters;
      split `submit()` and bring commands.rs (~800) / setup/mod.rs
      (~980) / llm.rs back under the 400-line cap; snap_dir() at all
      call sites; module-doc drift (tools.rs done, check others);
      remaining P2 hygiene (ASCII strings, busy indicator,
      IST vs Local snapshot dating, query_params support, usage/cost
      parsing, render examples for new cells). Atomic config.json and
      crm.toml writes: done 2026-10-09.

- [x] **/brief and /funnel open as tool rounds** (2026-10-09): the turn
      is the typed command, the tool call it stands for and that tool's
      output, so the model never re-calls the tool (it did: the brief
      printed twice, 6.5s -> 3.8s). One output per tool for both paths;
      the funnel tool takes `by`; build_brief_prompt is gone.

- [ ] **/ledger**: wire the prediction ledger view (stub today). Append
      predictions to `ledger.jsonl`, show hit-rate over time.
- [x] **Scheduled CRM fetch**: superseded (2026-10-09) - /schedule runs
      `moneyball refresh`, which pulls Meta then CRM into the same day.
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
