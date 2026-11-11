// The only file that calls invoke: one typed function per command in
// plans/desktop/IPC.md. Every rejection is normalised to AppError.
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  BriefView,
  By,
  FunnelView,
  RefreshEvent,
  RefreshState,
  ReportDay,
  ReportView,
  SessionSnapshot,
  SessionSummary,
  Today,
  TurnEvent,
  Workspace,
} from "./types";

export type * from "./types";

/** Whatever a command rejected with, as an AppError the UI can show. */
export function toAppError(e: unknown): AppError {
  if (e && typeof e === "object" && "reason" in e && typeof e.reason === "string") {
    const o = e as Partial<AppError>;
    return { kind: o.kind ?? "error", reason: e.reason, fix: o.fix ?? null };
  }
  return { kind: "error", reason: e instanceof Error ? e.message : String(e), fix: null };
}

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(cmd, args).catch((e: unknown) => {
    throw toAppError(e);
  });
}

function channel<T>(onEvent: (ev: T) => void): Channel<T> {
  const ch = new Channel<T>();
  ch.onmessage = onEvent;
  return ch;
}

export const workspaceCurrent = () => call<Workspace | null>("workspace_current");
export const workspacePick = () => call<Workspace | null>("workspace_pick");
export const today = () => call<Today>("today");
export const brief = () => call<BriefView>("brief");
export const funnel = (args: { product: string; days: 7 | 14 | 28; by: By }) => call<FunnelView>("funnel", args);
export const refreshStart = (onEvent: (ev: RefreshEvent) => void, force?: boolean) =>
  call<void>("refresh_start", { force, onEvent: channel(onEvent) });
export const refreshCancel = () => call<void>("refresh_cancel");
export const refreshState = () => call<RefreshState>("refresh_state");
export const reportDays = () => call<ReportDay[]>("report_days");
export const reportFor = (date?: string) => call<ReportView | null>("report_for", { date });
export const reportGenerate = (args: { date?: string; days: number }) => call<ReportView>("report_generate", args);
export const reportOpenInBrowser = (path: string) => call<void>("report_open_in_browser", { path });
export const sessionsList = () => call<SessionSummary[]>("sessions_list");
export const sessionCurrent = () => call<SessionSnapshot>("session_current");
export const sessionOpen = (id: string) => call<SessionSnapshot>("session_open", { id });
export const sessionNew = () => call<SessionSnapshot>("session_new");
export const turnStart = (text: string, onEvent: (ev: TurnEvent) => void) =>
  call<void>("turn_start", { text, onEvent: channel(onEvent) });
export const turnCancel = () => call<void>("turn_cancel");
export const copyText = (text: string) => call<void>("copy_text", { text });
