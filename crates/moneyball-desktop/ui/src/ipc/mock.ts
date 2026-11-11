// Dev and test only (never in the production bundle): serves the fixtures
// through @tauri-apps/api/mocks so the app runs in a plain browser.
// Flags come from the URL: ?mock=degraded,manual  (see NOTES.md).
import { mockIPC } from "@tauri-apps/api/mocks";
import * as fx from "./fixtures";
import type { AppError, RefreshEvent, RefreshState, Today, TurnEvent } from "./types";

type Chan<T> = { onmessage: (ev: T) => void };
type Args = Record<string, unknown>;

export type Mock = {
  flags: Set<string>;
  calls: [string, Args][];
  /** Push one event down the open refresh / turn channel. */
  refresh: (ev: RefreshEvent) => void;
  turn: (ev: TurnEvent) => void;
};

declare global {
  interface Window {
    __mock?: Mock;
  }
}

const noData: Today = {
  ...fx.today,
  health: { ...fx.today.health, snap_date: null, crm_snap_date: null, run: null, refreshed_at: null, state: { kind: "no_data" } },
  health_label: "No data",
  footer: ["Nothing pulled yet", ""],
  headline: { tint: "warn", text: "No data yet.", rest: "This workspace has no snapshot." },
  fixes: [{ kind: "refresh", label: "Refresh", does: "pulls Meta, the CRM and writes the first report", force: false }],
  day: null,
  last_run: null,
};

export function install() {
  const flags = new Set((new URLSearchParams(location.search).get("mock") ?? "").split(",").filter(Boolean));
  const calls: [string, Args][] = [];
  let refreshCh: Chan<RefreshEvent> | null = null;
  let turnCh: Chan<TurnEvent> | null = null;
  let refreshing: RefreshState | null = flags.has("running") ? fx.refreshStateRunning : null;
  let turnBusy = false;

  const fail = (e: AppError | undefined): never => {
    throw e;
  };
  const play = <T>(ch: Chan<T>, evs: T[], gap: number) =>
    evs.forEach((ev, i) => setTimeout(() => ch.onmessage(ev), gap * (i + 1)));

  const mock: Mock = {
    flags,
    calls,
    refresh: (ev) => {
      if (ev.type === "finished" || ev.type === "died" || ev.type === "could_not_start") refreshing = null;
      refreshCh?.onmessage(ev);
    },
    turn: (ev) => {
      if (["turn_complete", "turn_aborted", "failed", "died"].includes(ev.type)) turnBusy = false;
      turnCh?.onmessage(ev);
    },
  };
  window.__mock = mock;

  mockIPC((cmd, payload) => {
    const a = (payload ?? {}) as Args;
    calls.push([cmd, a]);
    const empty = flags.has("nodata");
    if (flags.has("fail") && ["today", "brief", "funnel", "report_days", "sessions_list"].includes(cmd)) fail(fx.errors.snapshot);
    switch (cmd) {
      case "workspace_current":
        if (flags.has("wsgone")) fail(fx.errors.no_workspace_config);
        return flags.has("noworkspace") ? null : fx.workspace;
      case "workspace_pick":
        return flags.has("pickcancel") ? null : fx.workspace;
      case "today":
        return empty ? noData : flags.has("degraded") ? fx.todayDegraded : fx.today;
      case "brief":
        return empty ? { ...fx.brief, brief: null, cpl_order: null, feasibility_line: null } : fx.brief;
      case "funnel":
        if (empty) return { ...fx.funnel, product: a.product, rows: [], cpl_ranking: null, notes: [] };
        return { ...(flags.has("degraded") ? fx.funnelNocrm : fx.funnel), product: a.product };
      case "refresh_state":
        return refreshing ?? fx.refreshState;
      case "refresh_start":
        if (refreshing) fail(fx.errors.refresh_busy);
        refreshCh = a.onEvent as Chan<RefreshEvent>;
        refreshing = fx.refreshStateWaiting;
        if (!flags.has("manual")) play(refreshCh, fx.refreshEvents, 700);
        if (!flags.has("manual")) setTimeout(() => (refreshing = null), 700 * fx.refreshEvents.length);
        return;
      case "refresh_cancel":
        return;
      case "report_days":
        return empty ? [] : fx.reportDays;
      case "report_for":
        return empty || flags.has("noreport") ? null : { ...fx.report, date: a.date ?? fx.report.date };
      case "report_generate":
        if (refreshing) fail(fx.errors.refresh_busy);
        return { ...fx.report, date: a.date ?? fx.report.date, days: a.days };
      case "sessions_list":
        return empty ? [] : fx.sessions;
      case "session_current":
        if (flags.has("markers")) return fx.sessionMarkers;
        return { ...(empty ? fx.sessionNew : fx.session), turn_active: flags.has("turnactive") };
      case "session_open":
        if (turnBusy) fail(fx.errors.busy);
        return a.id === fx.sessionMarkers.id ? fx.sessionMarkers : { ...fx.session, id: a.id };
      case "session_new":
        if (turnBusy) fail(fx.errors.busy);
        return fx.sessionNew;
      case "turn_start":
        if (String(a.text).startsWith("/")) fail(fx.errors.not_a_command);
        if (flags.has("nomodel")) fail(fx.errors.no_model);
        if (flags.has("busy") || turnBusy) fail(fx.errors.busy);
        turnCh = a.onEvent as Chan<TurnEvent>;
        turnBusy = true;
        if (!flags.has("manual")) play(turnCh, fx.turnEvents, 400);
        if (!flags.has("manual")) setTimeout(() => (turnBusy = false), 400 * fx.turnEvents.length);
        return;
      case "turn_cancel":
        if (turnBusy) setTimeout(() => mock.turn({ type: "turn_aborted" }), 50);
        return;
      case "report_open_in_browser":
      case "copy_text":
        return;
      default:
        throw { kind: "unknown_command", reason: `The mock has no command '${cmd}'.`, fix: null } satisfies AppError;
    }
  });
}
