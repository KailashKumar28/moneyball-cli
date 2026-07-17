# moneyball CRM contract - crm.json v1

moneyball is a read-only Meta-ads advisor. It joins ad spend (pulled from
Meta) with lead outcomes (from YOUR crm) to compute the funnel: spend ->
meta leads -> crm leads -> qualified -> visit -> booking.

This document is self-contained. If your CRM is custom-built (including
AI-built), paste this whole file into the coding agent that maintains your
CRM and ask it to "add an export that satisfies this contract". Then
iterate with the validator below until it passes.

## The file

One JSON file named `crm.json`, a flat array of ticket objects - one per
lead:

```json
[
  {
    "ad_id": "120211234567890123",
    "stage": "Contactable",
    "delivery": 1752624000,
    "funnel": "OPEN",
    "created_at": "2026-07-15T09:30:00+05:30"
  }
]
```

## Fields

| field        | required | type                          | meaning |
|--------------|----------|-------------------------------|---------|
| `ad_id`      | yes      | string                        | The Meta ad id this lead came from (from your lead-form webhook / Meta lead payload). This is the JOIN KEY - moneyball maps it to campaign and product via the ads snapshot. Must be the numeric Meta ad id, as a string. |
| `stage`      | yes      | string                        | Current pipeline stage of the lead. See stage names below. |
| `delivery`   | yes      | number or string              | When the lead was delivered to you. Epoch seconds (preferred), or an ISO-8601 datetime string with timezone. Date bucketing uses THIS field, never `created_at` - CRM record creation can lag delivery. |
| `funnel`     | no       | string                        | Set to `"WON"` when the lead converted (booking/purchase), regardless of stage. Anything else (or absent) means still open/lost. |
| `created_at` | no       | string                        | When the CRM record was created. Informational only. |

Extra fields are allowed and ignored. Do not omit a ticket because a field
is unknown - emit the ticket with the fields you have; only the three
required fields must be present and non-empty.

## Stage names

Canonical stages, in funnel order:

```
Lost  NonContactable  Contactable  Visit  Revisit  Booking
```

Semantics:

- qualified  = stage is `Contactable` or later
- visit      = stage is `Visit` or later
- booked     = stage is `Booking`, or `funnel` is `"WON"`

If your CRM uses different stage names, map them to these canonical names
inside your export (recommended - zero config on the moneyball side), or
configure a custom stage list in the workspace `config.json` under `crm.stages`.

## Where the file goes

Snapshots live per date at:

```
<workspace>/.moneyball/history/snap/<YYYY-MM-DD>/crm.json
```

next to `ads_daily.json` (written by `moneyball fetch`). `<YYYY-MM-DD>` is
the day the snapshot was taken; include ALL leads whose `delivery` falls in
at least the trailing 28 days (moneyball windows internally - a superset is
fine, a truncated export silently undercounts).

Two ways to deliver it:

1. **Write the file directly** - a daily cron/scheduler in your CRM writes
   `crm.json` into the NEWEST snapshot directory that already contains
   `ads_daily.json`. Never create a fresh date directory yourself: a
   snapshot dir with CRM data but no ads data makes `/brief` read zero
   spend for everything (moneyball's own `crm fetch` refuses to do this
   for the same reason). If today's dir is missing, either run
   `moneyball fetch` first or write into the newest dir that has ads.
2. **Expose an HTTP endpoint** - e.g. `GET /moneyball/crm.json` returning
   the array (optionally accepting `?days=N`). `moneyball crm fetch` pulls
   it on schedule.

## Validate

```
moneyball crm check path/to/crm.json
```

Checks shape, required fields, delivery parseability, stage names, and -
when a snapshot exists - what percentage of tickets join to a known Meta
ad id. Exit code 0 = PASS. Errors are precise and per-row; loop on them
until PASS.

## Rules that bite

- `ad_id` must be the Meta ad id. Lead-gen form ids, campaign ids, or your
  internal ids will not join - the validator's join-rate check catches this.
- Bucket by `delivery`, not `created_at` (the validator cannot see this,
  but the contract requires it: use the lead delivery timestamp).
- Emit strings for ids (JSON numbers lose precision at Meta's id sizes).
- The export is a full snapshot of current stage per lead, not an event
  log. If a lead moved from Visit to Booking, emit one ticket with stage
  `Booking`.
