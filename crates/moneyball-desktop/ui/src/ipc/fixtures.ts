// The drift alarm: every JSON fixture is checked against the contract's
// types here, so a shape change on either side fails `tsc`.
//
// A JSON import types "refresh" as string, which no literal union accepts,
// so `satisfies` checks against Loose<T>: the same shape with string
// literals widened. Field names, nesting, nullability and number-vs-string
// are all still checked.
import type * as T from "./types";

import briefJson from "./fixtures/brief.json";
import errorsJson from "./fixtures/errors.json";
import funnelJson from "./fixtures/funnel.json";
import funnelNocrmJson from "./fixtures/funnel_nocrm.json";
import refreshEventsJson from "./fixtures/refresh_events.json";
import refreshEventsOtherJson from "./fixtures/refresh_events_other.json";
import refreshFinishedDegradedJson from "./fixtures/refresh_finished_degraded.json";
import refreshStateJson from "./fixtures/refresh_state.json";
import refreshStateRunningJson from "./fixtures/refresh_state_running.json";
import refreshStateWaitingJson from "./fixtures/refresh_state_waiting.json";
import reportJson from "./fixtures/report.json";
import reportDaysJson from "./fixtures/report_days.json";
import sessionJson from "./fixtures/session.json";
import sessionMarkersJson from "./fixtures/session_markers.json";
import sessionNewJson from "./fixtures/session_new.json";
import sessionsJson from "./fixtures/sessions.json";
import todayJson from "./fixtures/today.json";
import todayDegradedJson from "./fixtures/today_degraded.json";
import turnEventsJson from "./fixtures/turn_events.json";
import turnEventsOtherJson from "./fixtures/turn_events_other.json";
import workspaceJson from "./fixtures/workspace.json";

type Loose<V> = V extends string
  ? string
  : V extends readonly (infer U)[]
    ? Loose<U>[]
    : V extends object
      ? string extends keyof V
        ? { [k: string]: Loose<V[string]> | undefined } // a JSON array of maps types absent keys as undefined
        : { [K in keyof V]: Loose<V[K]> }
      : V;

const as = <V>(v: Loose<V>) => v as unknown as V;

export const workspace = as<T.Workspace>(workspaceJson satisfies Loose<T.Workspace>);
export const today = as<T.Today>(todayJson satisfies Loose<T.Today>);
export const todayDegraded = as<T.Today>(todayDegradedJson satisfies Loose<T.Today>);
export const brief = as<T.BriefView>(briefJson satisfies Loose<T.BriefView>);
export const funnel = as<T.FunnelView>(funnelJson satisfies Loose<T.FunnelView>);
export const funnelNocrm = as<T.FunnelView>(funnelNocrmJson satisfies Loose<T.FunnelView>);
export const refreshState = as<T.RefreshState>(refreshStateJson satisfies Loose<T.RefreshState>);
export const refreshStateRunning = as<T.RefreshState>(refreshStateRunningJson satisfies Loose<T.RefreshState>);
export const refreshStateWaiting = as<T.RefreshState>(refreshStateWaitingJson satisfies Loose<T.RefreshState>);
export const refreshEvents = as<T.RefreshEvent[]>(refreshEventsJson satisfies Loose<T.RefreshEvent[]>);
export const refreshEventsOther = as<T.RefreshEvent[]>(refreshEventsOtherJson satisfies Loose<T.RefreshEvent[]>);
export const refreshFinishedDegraded = as<T.RefreshFinished>(
  refreshFinishedDegradedJson satisfies Loose<T.RefreshFinished>,
);
export const reportDays = as<string[]>(reportDaysJson satisfies string[]);
export const report = as<T.ReportView>(reportJson satisfies Loose<T.ReportView>);
export const sessions = as<T.SessionSummary[]>(sessionsJson satisfies Loose<T.SessionSummary[]>);
export const session = as<T.SessionSnapshot>(sessionJson satisfies Loose<T.SessionSnapshot>);
export const sessionNew = as<T.SessionSnapshot>(sessionNewJson satisfies Loose<T.SessionSnapshot>);
export const sessionMarkers = as<T.SessionSnapshot>(sessionMarkersJson satisfies Loose<T.SessionSnapshot>);
export const turnEvents = as<T.TurnEvent[]>(turnEventsJson satisfies Loose<T.TurnEvent[]>);
export const turnEventsOther = as<T.TurnEvent[]>(turnEventsOtherJson satisfies Loose<T.TurnEvent[]>);
export const errors = as<Record<string, T.AppError>>(errorsJson satisfies Record<string, Loose<T.AppError>>);
