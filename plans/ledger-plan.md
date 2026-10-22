# /ledger - plan

Status: built 2026-10-22 with the defaults under "Decisions" (each easy
to change: guards are constants in ledger/mod.rs). TODO.md "Next: /ledger". Written
2026-10-22 from mb.py's `rec submit / settle / ledger` (the reference
behavior) and what moneyball has since gained (diagnose, changes.json).

## What it is for

The advisor recommends ("cut the Lookalike ad set's budget 20%"). Today
those words vanish into the transcript; nobody learns whether they were
right. The ledger keeps each recommendation with the number it predicts
and the date to check it by, and settles it against the data when that
date comes. Over weeks it answers the only question that matters about
an advisor: how often is it right, by lever.

## Shape (mb.py, kept)

- A recommendation: product, entity (campaign / ad set / ad id), lever
  (budget, creative, targeting, structure, form, crm), the action in
  words, evidence (numbers it rests on, re-checked against the snapshot
  when submitted - mb.py rejects a rec whose evidence does not match
  the data within 5%), and a prediction: metric, direction, target,
  verify-by date.
- States: proposed -> applied | ignored | expired; applied -> confirmed
  | refuted | inconclusive. Append-only events in
  `.moneyball/ledger.jsonl` (a versioned artifact, moneyball.ledger/1).
- Guards: one open rec per entity+lever, a cooldown after an applied
  change (budget 2 days, creative 7, targeting / structure 14), at most
  5 new recs a day, budget moves above 20% need an explicit "accept a
  learning reset".

## What moneyball adds over mb.py

- "applied" is detected, not typed: changes.json now holds the account's
  change log, so a proposed budget rec whose ad set shows "Ad set budget
  updated" after it is offered as applied (mb.py printed candidates for
  a human to match).
- Settling is computed: at verify-by, the prediction's metric is
  re-derived from that day's snapshot (the same funnel / diagnose math)
  and the rec is marked confirmed / refuted, or inconclusive when the
  CRM data the metric needs is missing.
- The advisor submits through a `recommend` tool (validated like mb.py's
  submit: a rejection comes back to the model as a tool error it must
  fix), never by free text.

## Surfaces

- `/ledger` (TUI) and `moneyball ledger`: open recs, due recs, and the
  hit rate by lever over 30 / 90 days.
- The landing names recs due today ("2 predictions to check").
- report.json does not include the ledger (the client sees results,
  not the advisor's scorecard) - unless you want it to.

## Decisions (defaults taken; say if any should change)

1. You only: not in report.json or the client page.
2. Both: the change log marks budget, targeting, creative and structure
   changes applied; `/ledger <id> applied|ignored` for the rest (forms
   and the CRM never show in Meta's log).
3. mb.py's numbers as they were.
4. Yes, but a prediction or evidence about CRM metrics (crm_leads,
   qualified, cost_per_qualified, l_to_q) is refused while a snapshot
   has no CRM data; Meta-side ones (spend, meta_leads, cpl) are fine.

Settling: the metric over the full days from the day after the change
to the check date, counts as a 7-day rate (the table the advisor read
is 7 days); CRM metrics wait 3 more days for outcomes to land.

## The questions as asked

1. Is the ledger for you only, or does the client ever see it?
2. Who marks "applied": detection from the change log only, or may you
   also mark it by hand (`/ledger applied <id>`)?
3. Keep mb.py's guards as they are (5 a day, 20% budget step, the
   cooldowns), or different numbers?
4. Should the advisor be allowed to recommend at all before the CRM is
   back? (Without qualified counts most predictions can only be about
   Meta-side metrics: cost per lead, CTR, spend.)

Estimated size: core ledger module + tool + /ledger view + settle logic,
about 900 lines with tests; two to three focused sessions.
