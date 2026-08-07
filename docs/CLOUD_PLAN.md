# Cloud plan - creative report now, service later

Status: agreed direction 2026-08-07 (founder + expert review + three staff
plans). This file is the synthesis; it governs the creative-report slices
and constrains future cloud work. Prune as milestones ship.

## Verdict (unanimous across all four reviews)

**Path A: ship the daily creative report inside the CLI now, offline from
snapshots. The cloud product is a LATER, SEPARATE service (Postgres, web,
bots) that reuses moneyball-core as a crate.** Postgres never goes under
the CLI: it would delete the enforced read-path/network invariant, make
hermetic tests need a DB, and break the file contract external pipelines
rely on. Reference products (codex, pi, Claude Code) all keep the local
agent DB-free; their cloud variants are separate orchestrators running the
same core - that is the shape here.

Core principle: **report.json is the product; HTML is a renderer.** The
typed aggregate is computed once; browser, phone, WhatsApp, and Postgres
ingestion are all just renderers/consumers of the same artifact. This is
why nothing built now is throwaway.

## Phase 1 - the creative report (now)

Slices (re-cut per the data-contracts review; supersedes the first cut):

- **A0 - schema module + fixtures.** `schema.rs` serde structs for
  creatives.json + report.json with `schema: "moneyball.<artifact>/<major>"`
  fields; schemars-generated JSON Schemas committed to `docs/schemas/`;
  `tests/schema_contract.rs` pins them (a wire-shape edit fails CI until
  the schema is consciously re-committed). Hermetic fixture snapshot
  including creatives.json.
- **A1 - `/fetch` writes creatives.json.** Per ad: ad/adset/campaign ids,
  image_hash, video_id, afs_video_ids, image_basename, is_video, status,
  title/body/cta, permalink, image_url (never dereferenced at read time),
  asset {sha256, content_type, bytes}. Envelope object (`schema`,
  `fetched_at`, `rows`) - bare-array snapshots stay grandfathered v0.
  Network code stays in fetch.rs (or fetch/creatives.rs sibling).
- **A2 - content-addressed asset cache** (NOT `<ad_id>.jpg` - many ads
  share one creative): `history/assets/creatives/<hh>/<sha256>.<ext>`,
  temp-then-rename, download only if absent, failures non-fatal
  (`asset: null` -> placeholder card). Never delete during fetch.
- **B1 - `report.rs` compute -> report.json.** Grouping key computed at
  report time from stored facts (families/name-normalization are
  editorial policy, not snapshot facts): `family:` > `vidname:` (video
  name normalized, "- Copy N" stripped) > `vid:` > `img:<image_hash>` >
  `img:<basename>` > `ad:<ad_id>`, product-scoped. Funnel = the 7 canonical
  stages, always all present. Explicit `unattributed` bucket per product
  (Stattic-Ad-typo tickets never silently dropped). Reuse brief.rs
  Window/IST helpers - report and /brief must never disagree by a day.
  CLI prints a text summary (= the future bot renderer's dry run).
- **B2 - HTML renderer**, pure function over report.json + asset cache
  only (never snapshots), base64-inlined, self-contained. Output:
  `<workspace>/.moneyball/reports/<date>/creative-report.{json,html}`.
- **C - TUI `/report`**: thin wrapper, path + text summary as the cell.

Parity gate: golden-file harness diffing Rust report.json numerically
against the Python generator over the same archived snapshot dates. The
Python (`pipeline/creative_report.py`) is the behavioral spec - port its
domain rules (creative_key precedence, delivery-time bucketing, Stattic
Ad join, video_name_key for re-minted video ids); never import it.

Deliberate v1 exclusions (snapshot lacks the inputs; all additive later):
placement/gender/age breakdowns, targeting specs/learning stage,
activities timeline, re-inquiry flags.

Housekeeping shipped with A1: rename the global bug-report dir
`~/.moneyball/reports/` -> `~/.moneyball/bug-reports/` (collides with the
new workspace reports dir); update ARCHITECTURE section 2 write-boundary +
AGENTS Don'ts wording per the data-contracts review.

## Hedges applied now (cheap insurance for the service later)

1. **No ambient state in core**: core functions take explicit
   config/secrets parameters; nothing outside config.rs reads HOME or
   cwd. Enforce in arch_contract.rs. Concretely: `fetch_snapshot` takes
   the token as a param; add `WorkspaceConfig::from_parts`.
2. **Versioned, keyed artifacts**: every new JSON artifact carries
   `schema` + stable keys (workspace_id UUID minted into config.json,
   date, group_key) - the future idempotent-upsert keys.
3. **Seams stay sanctioned**: StreamFn/ToolExec/Ev are the only extension
   points; core stays tokio-free and terminal-free. The server crate must
   contain no copy of the loop (section 6b extended to bind it).

## Local data growth (decided 2026-08-07)

Files stay canonical locally at any size we will realistically see
(~1.5MB/day of JSON + a few MB of images; every current computation
reads one snapshot dir). When a feature needs CROSS-DATE queries
(multi-month trends, per-creative history beyond scoreboard.csv), add
SQLite as a DERIVED index - `.moneyball/state/index.db`, rebuilt from
the versioned artifacts at any time, deletable, never canonical. It
ingests the same keyed artifacts the cloud's Postgres ingests, so the
ingest logic is written once. No RAG/knowledge-base machinery for the
agent (section 6b minimalism): cross-date questions become one `trend`
tool over that index when the need is real, not before.

## Phase 2 - the service (later; build when a second real tenant asks)

Full details live in the 2026-08-07 planning transcripts; the binding
decisions:

- **Topology**: `moneyball-server` (axum + sqlx + in-process scheduler +
  agent runtime; core called via spawn_blocking - the loop is NOT
  async-ified) and later a thin `moneyball-botgw`. Server materializes
  per-tenant snapshot dirs and calls today's `snapshot::load` - the
  read-path contract survives verbatim in the cloud.
- **No bidirectional sync, ever.** Service fetches Meta/CRM independently
  (Postgres canonical in cloud); CLI stays file-canonical locally.
  Optional one-way `moneyball push` (artifact-file granularity,
  last-write-wins per (workspace, kind, date), never row-merge). Sessions
  never sync. Secrets never sync.
- **Sessions in Postgres** = one row per core `Item`, JSONB verbatim,
  append-only enforced by grants; replay + heal_history unchanged.
  "Exactly one in-flight turn" = status-column lease with 10-min
  staleness sweep (advisory lock only as same-node fast path). Bug
  reports: marker item + `bug_reports` row pointing at `seq <=
  item_count` (append-only makes the frozen copy unnecessary).
- **Channels**: Telegram first (zero approval friction, edit-in-place
  streaming, derisks pairing + turn machinery), then Slack, then WhatsApp
  (**start Meta business verification + template approval on day one -
  calendar-bound**), Teams last. Pairing: chat identity proves itself to
  the authenticated web session (code typed into the browser, never into
  the chat). Bots consume ItemDone/final only; web gets SSE deltas.
- **Daily brief push is NOT an agent run**: deterministic brief + one-shot
  commentary; WhatsApp as a utility template ("headline numbers + reply
  for details" - the reply opens the 24h window for the real conversation).
- **Tools in cloud**: brief/funnel via constructor-injected PgToolExec;
  add exactly `send_report_link` (server-composed signed URLs - the model
  never composes links). No send/action/sql tools - that refusal is a
  security control (prompt injection via ad copy/lead names has zero
  blast radius while all tools are read-only).
- **Cost policy**: strong model for all interactive turns; cheap model
  only for platform jobs (compaction summaries, brief commentary).
  Compaction (prompt-build substitution over untouched append-only
  history) when history > ~40k tokens; 24h-inactivity conversation
  rollover for bots. Token gauge metric from day one.
- **Deploy**: single DO Bangalore droplet + compose + Postgres, nightly
  pg_dump + assets to R2, ~$10-30/mo. AssetStore trait = the 10x seam.
- **Sequencing**: core seams/report port (L) -> server skeleton + ingest
  (M) -> reports over HTTP + magic-link auth (M) -> Telegram (S) ->
  advisor SSE + web chat (M) -> botgw + WhatsApp (M) -> multi-tenant
  hardening + push (M).

## Top risks (merged)

1. Report-math port fidelity (1.5k lines of domain subtlety) -> golden-file
   parity harness, port incrementally behind fixtures.
2. WhatsApp policy (templates, quality rating, unilateral changes) ->
   utility templates, hard opt-in/out, frequency cap, adapter-enforced
   window, Telegram/Slack fallback; per-tenant WABA or BSP at agency scale.
3. Core multi-tenant cleanliness ("reuse as crate" assumes no ambient
   state) -> hedge 1 above, applied now.
4. Meta token lifecycle in cloud (silent staleness) -> per-workspace
   health checks, last-good-fetch age in /healthz, ops alerts via own bot.
5. Scope creep toward sync/write features -> read-only guarantee enforced
   by arch-contract-style test in the server crate too; push-only
   artifact API; no row-level import endpoint exists.
