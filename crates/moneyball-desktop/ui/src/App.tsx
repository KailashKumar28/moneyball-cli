import { useCallback, useEffect, useRef, useState } from "react";
import { today, toAppError, workspaceCurrent, workspacePick, type Action, type AppError, type HealthState, type Workspace } from "./ipc";
import { clock, Glyph, sheetOpen, useAsync, useNow, type Async, type GlyphKind } from "./lib";
import { AddClient } from "./screens/AddClient";
import { Advisor } from "./screens/Advisor";
import { BriefScreen } from "./screens/Brief";
import { Empty } from "./screens/Empty";
import { FunnelScreen } from "./screens/Funnel";
import { RefreshScreen } from "./screens/Refresh";
import { ReportScreen, type ReportHandle } from "./screens/Report";
import { TodayScreen } from "./screens/Today";
import type { Today } from "./ipc";
import { useRefresh, type Refresh } from "./useRefresh";

export type Screen = "today" | "brief" | "funnel" | "report" | "advisor" | "refresh";
const NAV: [Screen, string][] = [
  ["today", "Today"],
  ["brief", "Brief"],
  ["funnel", "Funnel"],
  ["report", "Report"],
  ["advisor", "Advisor"],
];

export function App() {
  const current = useAsync(workspaceCurrent, []);
  const [picked, setPicked] = useState<Workspace | null>(null);
  const [pickError, setPickError] = useState<AppError | null>(null);
  const [adding, setAdding] = useState(false);
  // The client just made: it opens on its first refresh.
  const [created, setCreated] = useState<string | null>(null);
  const workspace = picked ?? current.data ?? null;

  const pick = useCallback(() => {
    workspacePick().then(
      (w) => {
        setPickError(null);
        if (w) {
          setPicked(w);
          setCreated(null);
        }
      },
      (e: unknown) => setPickError(toAppError(e)),
    );
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "o" && !sheetOpen()) {
        e.preventDefault();
        pick();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pick]);

  const add = useCallback(() => setAdding(true), []);
  const sheet = adding && (
    <AddClient
      onClose={() => setAdding(false)}
      onCreated={(w) => {
        setAdding(false);
        setPickError(null);
        setPicked(w);
        setCreated(w.path);
      }}
    />
  );
  if (!workspace || pickError)
    return (
      <>
        <Empty
          loading={current.loading}
          error={pickError ?? current.error}
          onAdd={add}
          onPick={pick}
          back={workspace && pickError ? { name: workspace.name, go: () => setPickError(null) } : null}
        />
        {sheet}
      </>
    );
  // A new workspace is a new app: every screen remounts and re-reads.
  return (
    <>
      <Shell key={workspace.path} workspace={workspace} onPick={pick} onAdd={add} created={created === workspace.path} />
      {sheet}
    </>
  );
}

type ShellProps = { workspace: Workspace; onPick: () => void; onAdd: () => void; created: boolean };

function Shell({ workspace, onPick, onAdd, created }: ShellProps) {
  const [screen, setScreen] = useState<Screen>(created ? "refresh" : "today");
  const [version, setVersion] = useState(0); // bumps when the data on disk changed
  const [newSession, setNewSession] = useState(0);
  const report = useRef<ReportHandle>(null);
  const bump = useCallback(() => setVersion((v) => v + 1), []);
  const td = useAsync(today, [version]);
  // Read whenever today is: setup in the terminal can rename the workspace or change its products.
  const ws = useAsync(workspaceCurrent, [version]);
  const shown = ws.data ?? workspace;
  const refresh = useRefresh(bump);

  // Coming back to the window: the terminal or the schedule may have changed any of it.
  const { reload } = td;
  const { reload: reloadWorkspace } = ws;
  const { sync } = refresh;
  useEffect(() => {
    const onFocus = () => {
      reload();
      reloadWorkspace();
      sync();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [reload, reloadWorkspace, sync]);

  const { start } = refresh;
  // Also a click handler, so anything but a literal true means "not forced".
  const startRefresh = useCallback(
    (force?: unknown) => {
      start(force === true);
      setScreen("refresh");
    },
    [start],
  );
  // A client just made has no data: its first refresh starts at once, on screen.
  const kicked = useRef(false);
  useEffect(() => {
    if (!created || kicked.current) return;
    kicked.current = true;
    startRefresh();
  }, [created, startRefresh]);
  const onAction = useCallback(
    (a: Action) => {
      if (a.kind === "refresh") startRefresh(a.force);
      if (a.kind === "report") {
        setScreen("report");
        report.current?.daily();
      }
    },
    [startRefresh],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.shiftKey || e.altKey || e.ctrlKey || sheetOpen()) return;
      const key = e.key.toLowerCase();
      const nav = NAV[Number(key) - 1];
      if (nav) setScreen(nav[0]);
      else if (key === "n") {
        setScreen("advisor");
        setNewSession((n) => n + 1);
      } else if (key === "r") startRefresh();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [startRefresh]);

  const shared = { onAction, version };
  return (
    <div className="frame">
      <aside className="side">
        <div className="dragzone" data-tauri-drag-region />
        <button className="ws" title="Open another workspace" onClick={onPick}>
          <b>{shown.name}</b>
          <span title={shown.display_path}>{shown.display_path}</span>
        </button>
        <button className="add" onClick={onAdd}>
          Add a Client...
        </button>
        <nav className="nav" aria-label="Views">
          {NAV.map(([id, label], i) => (
            <button key={id} className="navb" aria-current={screen === id ? "page" : undefined} onClick={() => setScreen(id)}>
              {label}
              <kbd>{"⌘"}{i + 1}</kbd>
            </button>
          ))}
        </nav>
        <div className="grow" />
        <HealthFooter today={td} refresh={refresh} current={screen === "refresh"} open={() => setScreen("refresh")} />
      </aside>
      <main>
        <TodayScreen hidden={screen !== "today"} today={td} {...shared} onRefresh={startRefresh} go={setScreen} />
        <BriefScreen hidden={screen !== "brief"} sub={td.data?.footer[0] ?? ""} {...shared} />
        <FunnelScreen hidden={screen !== "funnel"} products={shown.products} {...shared} />
        <ReportScreen hidden={screen !== "report"} handle={report} {...shared} onGenerated={bump} />
        <Advisor hidden={screen !== "advisor"} newSession={newSession} onAction={onAction} />
        <RefreshScreen hidden={screen !== "refresh"} refresh={refresh} onAction={onAction} go={setScreen} />
      </main>
    </div>
  );
}

const HEALTH_GLYPH: Record<HealthState["kind"], GlyphKind> = {
  fresh: "dot",
  crm_missing: "warn",
  partial: "warn",
  stale: "warn",
  pinned: "warn",
  no_data: "warn",
  failed: "bad",
};

/** How fresh the data is, and the way into Refresh. Shows the step in flight on every screen. */
function HealthFooter(props: { today: Async<Today>; refresh: Refresh; current: boolean; open: () => void }) {
  const { refresh, today: td } = props;
  const now = useNow(refresh.running);
  const step = refresh.steps.find((s) => s.status === "running");
  let glyph: GlyphKind = "idle";
  let lines: [string, string, string] = ["", "", ""];
  if (refresh.running) {
    glyph = "run";
    lines = ["Refreshing", step?.label ?? "", refresh.startedAt ? `${clock(now - refresh.startedAt)} so far` : ""];
  } else if (td.data) {
    glyph = HEALTH_GLYPH[td.data.health.state.kind] ?? "warn";
    lines = [td.data.health_label, ...td.data.footer];
  } else if (td.error) {
    glyph = "bad";
    lines = ["Unavailable", "", ""];
  }
  return (
    <button className="health" aria-current={props.current ? "page" : undefined} onClick={props.open} title="Open Refresh">
      <Glyph kind={glyph} />
      <b className="cap1">{lines[0]}</b>
      <span className="cap1">{lines[1]}</span>
      <span>{lines[2]}</span>
    </button>
  );
}
