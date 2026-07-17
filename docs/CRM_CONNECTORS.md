# moneyball CRM connectors - evidence catalog

Per-connector provenance for the presets in `crm::presets::catalog()`. Each
entry names the evidence source, the exact request shape it proved, and the
known gaps that need more evidence before the preset can be improved.

The contract itself lives in [CRM_CONTRACT.md](CRM_CONTRACT.md). This file
documents the WIRES - the request shape, paging strategy, and auth each
connector uses - so a future change can be verified against the same source
the preset was originally grounded in.

## LeadZump

Evidence source: `MOD_AI/fin_campaign_analysis/pipeline/fetch_leadzump.py`
(the only call site in the production pipeline; `creative_report.py`,
`weekly_funnel_report.py`, `monthly_funnel_report.py`, `mb.py` all import it).

### Endpoint and auth (verified)

- Base: `https://leadzump.ai/api/entity/processor`
- Paged list endpoint: `POST {base}/{entity}/eager/query?fetchOwners=false`
  - The `fetchOwners=false` query param trims a heavy join. Verified in
    `fetch_leadzump.py:45`.
- Headers (verified):
  - `appcode` (default `leadzump`, configurable via `LEADZUMP_APPCODE`)
  - `clientcode` (default `SYSTEM`, configurable via `LEADZUMP_CLIENTCODE`)
  - `authorization: <token>` (env `LEADZUMP_TOKEN` or `.env` `leadzump_token`)
  - `content-type: application/json`
- Token storage in moneyball: secret `leadzump_token` (per the preset).

### Body shape (verified)

The body is always:

```json
{
  "condition": <object-or-null>,
  "eager": true,
  "size": 200,
  "page": <int>
}
```

`condition` is either `null` (match everything) or a single object of the
form `{field, operator, value}` - NOT an array. Confirmed by every call site
in the pipeline. (`fetch_leadzump.py:46`, `creative_report.py:135`,
`weekly_funnel_report.py:82`, `monthly_funnel_report.py:70`, `mb.py:97`.)

The single operator the pipeline ever uses is `EQUALS`. Other operators are
NOT in the evidence base - any date-range / comparison operator is a guess
and must be verified against the live endpoint before shipping (see ARCHI-
TECTURE.md section 6: "Base URLs are verified against the live endpoint
before shipping").

### Response shape (verified)

The response is an object with at minimum:

```json
{
  "content": [ ... records ... ],
  "totalElements": <int>
}
```

`totalElements` is the total count the condition matched across all pages -
this is what the paging loop should consult to know when to stop. The
pipeline reads it (`fetch_leadzump.py:52`) and stops on
`len(rows) >= totalElements`.

`moneyball`'s preset does NOT currently read `totalElements`. It relies on
the empty-batch heuristic (`n < size`), which only works when the last page
is short. LeadZump returns full 200-record pages as long as more matches
exist, so the preset hits `MAX_PAGES=500` and bails. THIS IS THE ROOT
CAUSE OF THE LIVE-TEST-PULL RUNAWAY.

### Paging (verified)

- `page` is zero-indexed integer (pipeline starts at 0, moneyball preset
  matches with `start = 0`).
- `size` is the page size; the pipeline uses 200, moneyball matches.
- Termination rules from the pipeline (`fetch_leadzump.py:54`):
  1. empty batch, OR
  2. `len(rows) >= totalElements`, OR
  3. `page > 200` (safety backstop; moneyball uses 500).

### Mapping (verified)

Each record's relevant fields, confirmed by the pipeline's `funnel_for_-
campaign`:

| crm.toml map path         | LeadZump JSON path      | used in               |
|---------------------------|-------------------------|-----------------------|
| `ad_id` = `adId.adId`     | `adId.adId`             | moneyball only        |
| `stage` = `stage.name`    | `stage.name`            | both (`fetch_leadzump.py:67`) |
| `delivery`                | `delivery`              | moneyball only        |
| `funnel` = `status.funnelStage` | `status.funnelStage` | both (`fetch_leadzump.py:68`) |

AGENTS.md forbids editing the "Stattic Ad" Meta typo (it changes the join
key for LeadZump ad ids). Don't touch that.

### Known gaps

1. **Date filter not applied.** The preset's body has `"condition": null`
   and no `{from_date}` / `{to_date}` template substitution. Result: every
   pull returns the entire LeadZump ticket book, regardless of `--days`. The
   only filter the pipeline ever applies is `campaignId EQUALS <internal_id>`
   - that's a per-campaign pull from the funnel report, not a date window.

2. **The DSL operator for a date comparison is unverified.** The pipeline
   never uses a date comparison. Best-effort hypothesis (NOT to be shipped
   without verification):

   ```json
   {
     "condition": {"field": "delivery", "operator": "GREATER_THAN_EQUAL", "value": "<from_date>T00:00:00Z"},
     "eager": true, "size": 200, "page": 0
   }
   ```

   Verify by curling the live endpoint with this body, checking that the
   response's `totalElements` is bounded by the date window. If the operator
   vocabulary is different (e.g. `>=` literal, `BETWEEN`, etc.), update this
   doc with the actual shape.

3. **`totalElements` not consulted.** Once the date filter is in, the
   paging loop can also be tightened to stop at `records_so_far >=
   totalElements` - matches the pipeline's strategy and removes the need
   for the `MAX_PAGES=500` backstop on this connector.

### Connector upgrade checklist

When picking this up:

1. Update the preset's `body` template in `crates/moneyball-core/src/crm/presets.rs`
   to include the date condition (verify the operator against the live
   endpoint first; update this doc with the actual shape).
2. Add `totalElements` handling to `crm/fetch.rs` so the loop can terminate
   early on this connector (and any future connector that exposes it).
3. Update the preset's `note` to reflect what `--days` actually does.
4. Verify via `moneyball crm fetch --days 7` against the real endpoint:
   confirm per-page record counts shrink toward the end of the window, and
   the loop terminates without hitting `MAX_PAGES`.

## LeadSquared

Evidence source: LeadSquared's public API docs (linked in TODO.md: "Validate
LeadSquared for real"). The preset's exact field paths (`mx_*` attribution,
`ProspectStage`) follow the documented shape but have NOT been verified
against a live account. The live test at connect time gates this.

Document the request shape, paging, and verification checklist here once
the live validation is done.