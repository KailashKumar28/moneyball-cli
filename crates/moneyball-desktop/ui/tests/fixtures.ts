// What the tests expect, and where it comes from.
//
// `fx` is the generated set (src/ipc/fixtures/*.json, written by the shell
// from the Rust command bodies and pinned by a Rust test). Assertions derive
// from it, so regenerating the fixtures does not break a test.
//
// Below it are the hand-written states the generated set does not contain.
// They live here only, are typed against types.ts, and are listed in NOTES.md.
import { readFileSync } from "node:fs";
import type * as T from "../src/ipc/types";

const load = <V>(name: string): V =>
  JSON.parse(readFileSync(new URL(`../src/ipc/fixtures/${name}.json`, import.meta.url), "utf8")) as V;

export const fx = {
  workspace: load<T.Workspace>("workspace"),
  today: load<T.Today>("today"),
  todayDegraded: load<T.Today>("today_degraded"),
  brief: load<T.BriefView>("brief"),
  funnel: load<T.FunnelView>("funnel"),
  funnelNocrm: load<T.FunnelView>("funnel_nocrm"),
  refreshState: load<T.RefreshState>("refresh_state"),
  refreshStateDegraded: load<T.RefreshState>("refresh_state_degraded"),
  refreshStateRunning: load<T.RefreshState>("refresh_state_running"),
  refreshStateWaiting: load<T.RefreshState>("refresh_state_waiting"),
  refreshEvents: load<T.RefreshEvent[]>("refresh_events"),
  refreshEventsOther: load<T.RefreshEvent[]>("refresh_events_other"),
  refreshFinishedDegraded: load<T.RefreshFinished>("refresh_finished_degraded"),
  reportDays: load<T.ReportDay[]>("report_days"),
  report: load<T.ReportView>("report"),
  sessions: load<T.SessionSummary[]>("sessions"),
  session: load<T.SessionSnapshot>("session"),
  sessionNew: load<T.SessionSnapshot>("session_new"),
  sessionMarkers: load<T.SessionSnapshot>("session_markers"),
  turnEvents: load<T.TurnEvent[]>("turn_events"),
  turnEventsOther: load<T.TurnEvent[]>("turn_events_other"),
  errors: load<Record<string, T.AppError>>("errors"),
};
/** The first event of that type in a stream. */
export function event<E extends { type: string }, K extends E["type"]>(events: E[], type: K): Extract<E, { type: K }> {
  const ev = events.find((e) => e.type === type);
  if (!ev) throw new Error(`the fixtures have no '${type}' event`);
  return ev as Extract<E, { type: K }>;
}

// ---------- hand-written: states the generated fixtures lack ----------

const row = fx.funnel.rows[0]!;

/**
 * HAND-WRITTEN. Every verdict core can word (funnel/print.rs `verdict`): the
 * generated funnel has only keep and kill. Includes a real zero beside an
 * unknown, and a name far longer than the demo's.
 */
export const funnelAllVerdicts: T.FunnelView = {
  ...fx.funnel,
  rows: [
    ...fx.funnel.rows,
    { ...row, id: "h1", name: "Advantage+ Broad 25-44", spend: 9120, m: 14, l: 9, q: 0, v: 0, b: 0, cpl: 651, rs_per_q: null, l_to_q: 0, kill_mult: 3.6, kill: true, immature: true, verdict: "wait: immature" },
    { ...row, id: "h2", name: "Retargeting - site visitors 30d", spend: 1840, m: 3, l: 2, q: 1, v: 0, b: 0, cpl: 613, rs_per_q: 1840, l_to_q: 50, kill_mult: 0.7, kill: false, sufficient: false, immature: false, verdict: "wait: low spend" },
    { ...row, id: "h3", name: "Interest - Home loans and first-time buyers, Whitefield and Sarjapur Road corridor", spend: 6400, m: 0, l: 0, q: 0, v: 0, b: 0, cpl: null, rs_per_q: null, l_to_q: null, kill_mult: 2.5, kill: false, immature: false, learning: "LEARNING", verdict: "wait: learning" },
  ],
};

/** HAND-WRITTEN. A workspace that has never pulled: what each command returns then (words from core's landing story). */
export const noData: Record<string, unknown> = {
  today: {
    ...fx.today,
    health: { ...fx.today.health, snap_date: null, crm_snap_date: null, run: null, refreshed_at: null, state: { kind: "no_data" } },
    health_label: "No data",
    footer: ["No data yet", ""],
    headline: { tint: "warn", text: "No data yet.", rest: "Nothing has been pulled for this workspace." },
    next: { kind: "refresh", label: "Refresh", does: "pulls yesterday's numbers and writes the report", force: false },
    day: null,
    last_run: null,
  } satisfies T.Today,
  brief: { brief: null, crm_head: null, crm_note: null, fault: null, staleness: null, cpl_order: null, caption: null, feasibility_line: null } satisfies T.BriefView,
  funnel: { product: fx.workspace.products[0]!, has_crm: true, rows: [], cpl_ranking: null, notes: [] } satisfies T.FunnelView,
  refresh_state: { running: false, day: "", steps: [], started_ms_ago: null, finished: null } satisfies T.RefreshState,
  report_days: [] satisfies T.ReportDay[],
  report_for: null,
  sessions_list: [] satisfies T.SessionSummary[],
  session_current: fx.sessionNew,
};

const finished = event(fx.refreshEvents, "finished");
const [meta, assets, crm, report] = fx.refreshFinishedDegraded.run.steps as [T.StepView, T.StepView, T.StepView, T.StepView];
const crmFix: T.Action = { kind: "terminal", command: "moneyball crm secret acme_crm_token", does: "paste the new key; it is stored in the keychain, never in the workspace" };

/**
 * HAND-WRITTEN. A refresh whose CRM step failed with a sentence, not counts:
 * the generated runs have no failed step. The message is as long as a real
 * HTTP failure is.
 */
export const refreshCrmFailed: T.RefreshFinished = {
  ...fx.refreshFinishedDegraded,
  run: {
    ...fx.refreshFinishedDegraded.run,
    steps: [
      meta,
      assets,
      { ...crm, status: "failed", word: "failed", detail: "the CRM answered 401 Unauthorized: the key was rejected, or it has been rotated since setup", time: "0.8s", fix: crmFix },
      report,
    ],
  },
  verdict: { tint: "warn", headline: "The report went out without CRM.", rest: "CRM: the CRM answered 401 Unauthorized: the key was rejected, or it has been rotated since setup", next: null },
  fixes: [crmFix],
};

/**
 * HAND-WRITTEN. A second refresh on the same day: Meta and the assets come
 * from the earlier pull and carry the shell's `note` (run_view.rs, core's
 * REUSED), which no generated fixture has.
 */
export const REUSED = "from today's earlier pull";
export const refreshReused: T.RefreshFinished = {
  ...finished,
  time: "11s",
  run: {
    ...finished.run,
    steps: finished.run.steps.map((s) => (s.name === "meta" || s.name === "assets" ? { ...s, note: REUSED, time: null } : s)),
  },
};
/** HAND-WRITTEN. Today after that run: the same reused steps in the Data health table. */
export const todayReused: T.Today = { ...fx.today, last_run: refreshReused.run };

/** HAND-WRITTEN. The last recorded run died or is running elsewhere: its record is still marked running, so there are steps and no verdict. */
export const refreshNeverFinished: T.RefreshState = {
  ...fx.refreshState,
  steps: fx.refreshStateRunning.steps.map((s) => (s.status === "running" ? fx.refreshStateWaiting.steps.find((w) => w.name === s.name)! : s)),
  finished: null,
};

/** HAND-WRITTEN. The day's report is missing and Today offers to generate it (core's landing, DayReport::Missing). */
export const todayReportMissing: T.Today = {
  ...fx.today,
  day: {
    ...fx.today.day!,
    numbers: null,
    versus: null,
    report_missing: { note: "no daily report for this day yet", next: { kind: "report", label: "Generate report", does: "builds it from the snapshot" } },
  },
};

/** HAND-WRITTEN. The data day is past the 28th: the shell offers the longest window a snapshot holds in place of the month. */
export const reportDaysLateMonth: T.ReportDay[] = fx.reportDays.map((d) => ({
  ...d,
  windows: [...d.windows.slice(0, 2), { label: "Last 28 days", days: 28 }],
}));

/** HAND-WRITTEN. A turn failed while the window was reloading: only the snapshot says so. */
export const sessionFailed: T.SessionSnapshot = { ...fx.session, failed: event(fx.turnEventsOther, "failed").error };

/** HAND-WRITTEN. The workspace after setup ran in the terminal: renamed, with one more product. */
export const workspaceChanged: T.Workspace = { ...fx.workspace, name: "acme-homes-2", products: [...fx.workspace.products, "Lakeside"] };
