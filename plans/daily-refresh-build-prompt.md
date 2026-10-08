# Build prompt: moneyball daily refresh and data health

You are building the **daily refresh** for moneyball-cli (`/Users/Kailash/DEV/moneyball-cli`): one command that pulls Meta, then CRM, then writes yesterday's client report, a schedule that runs it every morning, and screens that say how fresh the data is before any number does. Part A is the core contract, Part B the screens. Do Part A first, and **ship A1, the verdict gate, before anything else**: today the product hands out Kill verdicts computed from missing CRM data, including in the client report.

The target design is the flow board's sections **R** (Redesign: daily refresh, frames R1 to R6) and **S** (when a refresh goes wrong, S1 to S5) in `plans/prototype/board.html` in this repo. Sections A to F are today's product, captured from the installed binary, with the evidence for everything below. Each frame's **Copy for agent** gives its exact cells, every styled run with its ratatui `Style`, and the build notes; its Build plan tab orders the tasks.

Read the repo's `CLAUDE.md`, `AGENTS.md` and `ARCHITECTURE.md` first. They are binding: the network boundary, E2E first, `cargo install` is shipping, ASCII-only authored TUI strings, never advertise a command before it works.

The pick came from a three-engineer review panel on 8 Oct 2026 (user journey, data and operations, advisor and architecture). Two picked the daily refresh; the third picked trustworthy verdicts first. All three put the verdict gate first, which is why it is A1 here.

---

## 0. What is wrong today (measured 8 Oct 2026, code at 500e66a)

1. **Nothing runs on its own.** The python pipeline it replaced ran under launchd at 07:30 and last ran on 18 Jul. Since then the workspace has 6 snapshots in 82 days (7, 8, 9, 10 and 12 Aug, 28 Sep), a 46-day hole, and nothing after 28 Sep. Only 3 daily reports have CRM data. TODO.md calls a cron recipe "existing"; none exists.
2. **CRM is a separate, order-dependent shell step.** `/fetch` pulls Meta only; `moneyball crm fetch` is shell-only. It files into the newest snapshot dir with ads but windows on the pull date (`crm/fetch.rs:78-79, 266-285`). Run before the Meta pull, it lands on yesterday: 8 Aug needed a re-run, and 28 Sep never got CRM at all.
3. **Missing CRM produces confident false verdicts.**
   - `moneyball funnel "Disha Namma Mane"` on 28 Sep: `kill=true` on the three biggest ad sets (Rs 85k, 50k, 25k) because qualified is 0, not because they failed (`funnel/mod.rs:157-158`).
   - The 27 Sep client report has 11 Kill and 11 "0 CRM" chips next to a box saying CRM is missing (`report/targeting.rs` verdicts() has no CRM guard).
   - A real session on 8 Aug turned the same table into "KILL all three".
   - The kill target defaults to Rs 2,500 per qualified, which nobody chose; real cost per qualified is Rs 5,409 to 60,509.
4. **Nothing says the data is stale.**
   - The bar shows a green "● ready" on any data.
   - Only `/brief` mentions age.
   - The HTML's "pulled" is the generation time.
   - The prior comparison reaches back 47 days.
   - NO_CRM_WARNING says "connect a CRM" when it is connected and just not fetched (`crm/mod.rs:62`).
   - A real session on 28 Sep: the advisor planned on 47-day-old numbers until the user wrote "this is very old data".
5. **Expired tokens are dead ends.**
   - Meta code 190 comes through raw ("code Some(190)", `meta.rs:98`).
   - `/setup` on a configured workspace jumps to the model step, so the token step is unreachable.
   - `moneyball fetch` with no token says "run /setup" from the shell.
   - A CRM 401 prints a literal `<name>` placeholder.
   - One failing ad account aborts every product (`fetch/mod.rs:79-86`).

---

## Part A - Core (moneyball-core)

### A1. The verdict gate (ship first)

No CRM data, no verdict. When the snapshot has no CRM data:
- Kill, scale and 0-CRM are withheld in the funnel rows, the report's targeting verdicts, and the agent's funnel tool.
- They are never computed from zeros.

| Change | Where |
|---|---|
| `FunnelRow.kill: Option<bool>`, `None` without CRM | `funnel/mod.rs` |
| `verdicts()` returns one `Verdict{code:"no_crm", ...}` with the reason | `report/targeting.rs` |
| The tool output says "verdicts withheld: no CRM data (last pull <date>)" | `moneyball-tui/src/commands.rs` funnel tool |

These changes are additive within `moneyball.creative_report/1`. Regenerate `docs/schemas` with `MB_UPDATE_SCHEMAS=1 cargo test`.

### A2. One CRM health type

```rust
enum CrmHealth { NoSpec, NoData, Stale { last: NaiveDate }, Fresh { joined_pct: u8 } }
```

One `note()` gives the one sentence and the one fix. It replaces:
- the NO_CRM_WARNING constant;
- the exec brief's wording;
- `crm status`'s next line, which today wrongly says "today's snapshot".

### A3. Typed errors and token recovery

| Error | When | Fix line |
|---|---|---|
| `MetaAuth { expired_on }` | Meta code 190 | `moneyball meta token` |
| `MetaPermission { account }` | codes 10, 200, 294 | grant the system user access to `<account>` |
| `CrmAuth { secret_name }` | HTTP 401/403 | `moneyball crm secret <the spec's real secret name>` |

`moneyball meta token` is new:
- It reads the token from stdin, the same pattern as `crm secret`.
- It validates it with `/me` and stores it in `~/.moneyball/auth.json` (0600).
- The token is never echoed and never put in argv.

### A4. One account never aborts the pull

Fetch insights per ad account. A failing account marks its product "no data: <reason>"; the other products land.

### A5. CRM anchored to the snapshot day

`fetch_crm(cfg, date)` windows tickets to that snapshot's IST day and writes into that dir only, so CRM always matches the same day's Meta pull.

### A6. Run records

`.moneyball/runs/<date>.json`, schema `moneyball.run/1` (serde struct in `schema/`, schemars file in `docs/schemas/`, pinned by `schema_contract`):

| Field | Type | Notes |
|---|---|---|
| `schema` | str | `moneyball.run/1` |
| `workspace_id`, `run_id` | str | |
| `date` | date | the snapshot date; the data covers the day before |
| `trigger` | `schedule` / `manual` | |
| `started_at`, `finished_at` | RFC3339 | |
| `outcome` | `ok` / `degraded` / `failed` | failed if Meta failed; degraded if CRM failed or a product is missing |
| `steps[]` | `{name, status, ms, counts, error_kind, message, fix}` | `name`: meta, assets, crm, report. `status`: done, failed, skipped, partial |
| `artifacts` | map | files the run wrote |
| `meta_token_expires` | date or null | from Meta's debug_token (decision 6) |

A lock at `.moneyball/state/refresh.lock` allows one run at a time per workspace.

### A7. `refresh::run`

A new `refresh.rs` that orchestrates and has **no network code of its own**: it calls `fetch_snapshot`, `fetch_crm` and `report::generate`, the existing network modules, so `arch_contract` is unchanged.
- **Steps:** Meta, then creatives, leads and adsets, then CRM (skipped if Meta failed, so CRM never lands on a different day), then the report.
- **Idempotent per day:** today's Meta pull is reused unless `--force`, so a second run after fixing a CRM token only re-runs CRM and the report.
- **The record is written even when a step fails.**
- **Exit codes:** 0 ok, 2 degraded, 1 failed.

### A8. Report provenance

`report.json` `source` gains `meta_fetched_at`, `crm_fetched_at` and `crm_status`, additive in `/1`.
- The HTML freshness line says when the data was pulled, not when the report was generated.
- `load_prior` ignores reports older than 7 days (decision 5).

### A9. Status

`runs::recent(cfg, 14)` reads the run records and turns gaps into "missed" rows.

### A10. Schedule

A per-workspace launchd agent:
- **Label:** `ai.moneyball.refresh.<workspace id, 8 chars>`.
- **ProgramArguments:** the absolute binary path, `refresh`, `--data-root <workspace>`.
- **StartCalendarInterval:** the run time (default 07:30).
- **WorkingDirectory:** the workspace; stdout and stderr go into `.moneyball/runs/`.

launchd runs a missed calendar job when the Mac wakes. Writing `~/Library/LaunchAgents` is outside today's write boundary (decision 1).

### A11. Rules for this code

- Network stays in the four allowed modules; the orchestrator only calls them.
- Schemas are serde structs in `schema/` with a generated JSON Schema; changes are additive within a major.
- New TUI strings are ASCII; reuse the glyphs the TUI already paints.
- Tests: table-driven unit tests per outcome on a fake snapshot dir, and the schema contract.

---

## Part B - Screens (moneyball-tui and the CLI)

Match the frames cell for cell; their Copy for agent lists every styled run.

### B1. The context bar and the landing (R1, R5)

- **Bar:** `data <day>  ·  CRM <day>  ·  ● <health>` in place of `● ready`. Labels are DarkGray, values Gray, and health is coloured:
  - Green: fresh.
  - Yellow: stale (2+ days), CRM missing, or partial.
  - Red: failed, or 7+ days old.
  - Cyan: refreshing.
- **Landing, not fresh:** one notice cell (what is wrong, since when, the fix) with `/refresh` and `/schedule` in Cyan bold.
- **Landing, fresh:** yesterday's line, the change against the last day within 7, the report's full path, and "o open it".
- **Delete:** `Status::Loading`, `Status::Error` and `widgets::welcome_text` (all unused).

### B2. `/refresh` (R2, R3, S1 to S3)

- **Worker:** a worker thread with its own channel, not the shared `app.stream` slot, so the chat answers while it runs. Esc cancels after the current step.
- **Tool cell:** `⎿ ◐ moneyball refresh`, then a step table (number, name, state, counts, time). States: done Green, running Yellow, failed Red, skipped DarkGray.
- **Done:** the summary line, the full report path (wrapped, never clipped) and "o open it", plus the `/schedule` offer if none is installed.
- **Degraded and failed:** a Yellow or Red first sentence, what was and was not written, and one `fix:` line with the exact command.
- **`/fetch`:** becomes an alias of `/refresh` (decision 7).

### B3. `moneyball refresh` (S5)

- One plain line per step, then `outcome:`, `fix:`, `report:` and `run record:`.
- Exit code 0, 2 or 1.
- With `MB_AGENT=1`, the run record is printed as JSON.

### B4. `/status` and `moneyball status` (R6)

- 14 rows, one per data day: Meta, CRM, leads, report, and when and how it ran. Missed days are rows.
- Footer: the schedule, token health ("Meta valid to <date>"), and the records path.

### B5. `/schedule` and `moneyball schedule [--at HH:MM] [--off] [--print]` (R4)

- A confirm cell names what it writes, what it runs, where it logs, and how to remove it, before writing anything.
- Keys: `y` install, `t` change the time, `p` print the plist instead, Esc cancel.

### B6. "no CRM" where a verdict was (S4)

`/funnel`, the CLI funnel and the report's chips show "no CRM" in Yellow, then one line with the last CRM pull and `/refresh`.

---

## Out of scope

- **Backfilling missed days as daily reports:** CRM stages are today's, not that day's (decision 3).
- **OS notifications and cloud alerts** (Phase 2 in docs/CLOUD_PLAN.md).
- **LLM commentary in the daily run.** CLOUD_PLAN: the daily push is not an agent run.
- **The onboarding CRM step, the wizard paste bug and the advisor's model policy:** separate work. The paste bug (`event.rs:216-222`) should still ship as a plain fix.

## Open decisions (defaults used above)

1. **Who installs the schedule.** Default: `moneyball schedule` writes `~/Library/LaunchAgents/...plist` after an explicit confirm, and AGENTS.md's write boundary gains that one file. Alternative: `--print` only.
2. **CRM fails, Meta works.** Default: write the report with CRM hidden and say so. Alternatives: hold the report, or reuse the last good CRM labelled with its date.
3. **Missed days.** Default: show them as rows in `/status`, no backfill.
4. **LLM after a refresh.** Default: none; commentary on demand.
5. **Prior comparison horizon.** Default: 7 days, else "no comparable day in the last 7".
6. **Meta token expiry.** Default: read `expires_at` from debug_token on each refresh and warn 7 days ahead.
7. **`/fetch`.** Default: an alias of `/refresh`.
8. **Run time.** Default: 07:30 IST.

## Done when (on the installed binary, the real test workspace)

- **No verdict from missing CRM.**
  - `moneyball report --date 2026-09-28` has no Kill or 0-CRM chip.
  - `moneyball funnel "Disha Namma Mane"` on 28 Sep prints "no CRM".
  - The same holds for `/funnel` in tmux.
- **A real refresh, end to end.**
  - `moneyball refresh` pulls Meta and LeadZump, writes the report with CRM and a valid run record, and exits 0.
  - With the CRM secret removed, it exits 2, the report has CRM hidden, and the fix line names `leadzump_token`.
- **The landing.** It matches R1 on the 28 Sep snapshot and R5 after a refresh.
- **Status.** `/status` lists the missed days as rows.
- **The schedule.** `moneyball schedule --at <2 minutes ahead>` fires, the run record appears, and `--off` removes the plist.
- **The board.** `capture.py` recaptured with the new binary: frames F3 and F4 show the gate.
- **The gates.** `cargo test` and clippy are green, `arch_contract` is green, and the binary is reinstalled with `cargo install`.
