// Dev and test only (never in the production bundle): serves the fixtures
// through @tauri-apps/api/mocks so the app runs in a plain browser.
// Flags come from the URL: ?mock=degraded,manual  (see NOTES.md).
// Everything it serves is a generated fixture; a test that needs a state
// the fixtures lack passes its own reply for a command (window.__replies).
import { mockIPC } from "@tauri-apps/api/mocks";
import * as fx from "./fixtures";
import type { AppError, RefreshEvent, RefreshState, SessionSnapshot, TurnEvent } from "./types";

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
    /** Set before the page loads: a fixed reply per command, served instead of the fixture. */
    __replies?: Record<string, unknown>;
  }
}

export function install() {
  const flags = new Set((new URLSearchParams(location.search).get("mock") ?? "").split(",").filter(Boolean));
  const calls: [string, Args][] = [];
  let refreshCh: Chan<RefreshEvent> | null = null;
  let turnCh: Chan<TurnEvent> | null = null;
  let refreshing: RefreshState | null = flags.has("running") ? fx.refreshStateRunning : null;
  let turnBusy = false;
  // The shell's session: no id until its first turn creates the file.
  let current: SessionSnapshot = flags.has("markers") ? fx.sessionMarkers : fx.session;
  const replies = window.__replies ?? {};

  const fail = (e: AppError): never => {
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
    if (cmd in replies) return replies[cmd];
    if (flags.has("fail") && ["today", "brief", "funnel", "report_days", "sessions_list"].includes(cmd)) fail(fx.errors.snapshot);
    switch (cmd) {
      case "workspace_current":
        if (flags.has("wsgone")) fail(fx.errors.no_workspace_config);
        return flags.has("noworkspace") ? null : fx.workspace;
      case "workspace_pick":
        return flags.has("pickcancel") ? null : fx.workspace;
      case "today":
        return flags.has("degraded") ? fx.todayDegraded : fx.today;
      case "brief":
        return fx.brief;
      case "funnel":
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
        return fx.reportDays;
      case "report_for":
        return flags.has("noreport") ? null : { ...fx.report, date: a.date ?? fx.report.date };
      case "report_generate":
        if (refreshing) fail(fx.errors.refresh_busy);
        return { ...fx.report, date: a.date ?? fx.report.date, days: a.days };
      case "sessions_list":
        return fx.sessions;
      case "session_current":
        return { ...current, turn_active: flags.has("turnactive") };
      case "session_open":
        if (turnBusy) fail(fx.errors.busy);
        current = a.id === fx.sessionMarkers.id ? fx.sessionMarkers : fx.session;
        return current;
      case "session_new":
        if (turnBusy) fail(fx.errors.busy);
        current = fx.sessionNew;
        return current;
      case "turn_start":
        if (String(a.text).startsWith("/")) fail(fx.errors.not_a_command);
        if (flags.has("nomodel")) fail(fx.errors.no_model);
        if (flags.has("busy") || turnBusy) fail(fx.errors.busy);
        turnCh = a.onEvent as Chan<TurnEvent>;
        turnBusy = true;
        // the first turn of a new session is the one turn_events.json records
        if (current.id === null) current = { ...current, id: fx.session.id };
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
