// tsc checks the fixtures' shape (src/ipc/fixtures.ts) but cannot see a
// literal: a JSON import types "done" as string. This checks every literal
// the UI switches on, in every generated fixture, against the unions in
// types.ts. (The shell once sent `done` where the contract said `ok`.)
import { expect, test } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import type * as T from "../src/ipc/types";

const DIR = new URL("../src/ipc/fixtures/", import.meta.url);

/** Every member of a union, and nothing else: Record<U, true> fails tsc on a missing or a stray key. */
const members = <U extends string>(all: Record<U, true>) => Object.keys(all);

type Obj = Record<string, unknown>;
type Rule = { key: string; what: string; allowed: string[]; file?: RegExp; when?: (o: Obj, parent: string) => boolean };

const RULES: Rule[] = [
  { key: "tint", what: "Tint", allowed: members<T.Tint>({ plain: true, good: true, warn: true, bad: true }) },
  { key: "kind", what: "Action.kind", when: (o) => "does" in o, allowed: members<T.Action["kind"]>({ refresh: true, report: true, terminal: true }) },
  {
    key: "kind",
    what: "HealthState.kind",
    when: (_, parent) => parent === "state",
    allowed: members<T.HealthState["kind"]>({ failed: true, no_data: true, pinned: true, stale: true, crm_missing: true, fresh: true, partial: true }),
  },
  {
    key: "status",
    what: "StepView.status",
    when: (o) => "word" in o,
    allowed: members<T.StepView["status"]>({ done: true, partial: true, failed: true, skipped: true, waiting: true, running: true }),
  },
  { key: "name", what: "StepView.name", when: (o) => "word" in o, allowed: members<T.StepView["name"]>({ meta: true, assets: true, crm: true, report: true }) },
  { key: "status", what: "RunStep.status", when: (o) => "counts" in o, allowed: members<T.RunStep["status"]>({ done: true, partial: true, failed: true, skipped: true }) },
  { key: "status", what: "ToolStatus", when: (o) => "call_id" in o, allowed: members<T.ToolStatus>({ pending: true, ok: true, failed: true }) },
  { key: "trigger", what: "trigger", allowed: members<T.RunView["trigger"]>({ manual: true, schedule: true }) },
  { key: "source", what: "DayNumber.source", allowed: members<NonNullable<T.DayNumber["source"]>>({ meta: true, crm: true }) },
  { key: "outcome", what: "RunRecord.outcome", when: (o) => "schema" in o, allowed: members<T.RunRecord["outcome"]>({ running: true, ok: true, degraded: true, failed: true }) },
  {
    key: "type",
    what: "RefreshEvent.type",
    file: /^refresh_/,
    allowed: members<T.RefreshEvent["type"]>({ begin: true, done: true, finished: true, could_not_start: true, died: true }),
  },
  {
    key: "type",
    what: "TurnEvent.type",
    file: /^turn_events/,
    when: (_, parent) => parent !== "item",
    allowed: members<T.TurnEvent["type"]>({
      assistant_delta: true, assistant_done: true, tool_begin: true, tool_end: true, item_done: true,
      turn_complete: true, turn_aborted: true, failed: true, died: true, save_failed: true,
    }),
  },
  {
    key: "type",
    what: "Item.type",
    file: /^turn_events/,
    when: (_, parent) => parent === "item",
    allowed: members<T.Item["type"]>({ user: true, assistant: true, tool_call: true, tool_output: true }),
  },
  {
    key: "type",
    what: "ViewItem.type",
    file: /^session/,
    allowed: members<T.ViewItem["type"]>({ user: true, assistant: true, tool: true, turn_interrupted: true, bug_report_filed: true }),
  },
];

test("every literal in the generated fixtures is one types.ts names", () => {
  const unknown: string[] = [];
  const seen = new Set<Rule>();
  const walk = (file: string, v: unknown, parent: string) => {
    if (Array.isArray(v)) return v.forEach((x) => walk(file, x, parent));
    if (!v || typeof v !== "object") return;
    const o = v as Obj;
    for (const rule of RULES) {
      if (!(rule.key in o) || (rule.file && !rule.file.test(file)) || (rule.when && !rule.when(o, parent))) continue;
      seen.add(rule);
      if (!rule.allowed.includes(String(o[rule.key]))) unknown.push(`${file}.json: ${rule.what} "${String(o[rule.key])}"`);
    }
    for (const [k, x] of Object.entries(o)) walk(file, x, k);
  };
  for (const name of readdirSync(DIR)) walk(name.replace(/\.json$/, ""), JSON.parse(readFileSync(new URL(name, DIR), "utf8")), "");
  expect([...new Set(unknown)]).toEqual([]);
  // a rule that matches nothing has rotted: the fixtures moved and it checks nothing
  expect(RULES.filter((r) => !seen.has(r)).map((r) => r.what)).toEqual([]);
});
