// One refresh at a time, owned by the app so the sidebar footer can show it
// on every screen. Core reports only a step beginning and ending; the
// elapsed times are this window's own clock. At rest everything shown is
// what refresh_state returns (the workspace's last recorded run), so a
// reload, a workspace switch or a run by the terminal loses nothing.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  refreshCancel,
  refreshStart,
  refreshState,
  toAppError,
  type AppError,
  type Finished,
  type RefreshEvent,
  type StepView,
} from "./ipc";

export type Refresh = {
  running: boolean;
  day: string;
  steps: StepView[];
  startedAt: number | null;
  stepStartedAt: number | null;
  finished: Finished | null;
  error: AppError | null; // could not start, or the state would not load
  died: string | null;
  cancelling: boolean;
  start: (force?: boolean) => void;
  cancel: () => void;
  /** Re-read the state: the terminal or the schedule may have run since. */
  sync: () => void;
};

type State = Omit<Refresh, "start" | "cancel" | "sync">;

const idle: State = {
  running: false,
  day: "",
  steps: [],
  startedAt: null,
  stepStartedAt: null,
  finished: null,
  error: null,
  died: null,
  cancelling: false,
};

const upsert = (steps: StepView[], step: StepView) =>
  steps.some((s) => s.name === step.name) ? steps.map((s) => (s.name === step.name ? step : s)) : [...steps, step];

export function useRefresh(onSettled: () => void): Refresh {
  const [st, setSt] = useState<State>(idle);
  const settled = useRef(onSettled);
  const running = useRef(false);
  const attached = useRef(false); // this window holds the run's channel
  useEffect(() => {
    settled.current = onSettled;
    running.current = st.running;
  });

  // Mount, reload and focus: a run may be in flight with no channel to it, or one made elsewhere may be the last.
  const sync = useCallback(async () => {
    try {
      const s = await refreshState();
      if (attached.current) return;
      const was = running.current;
      setSt((p) => ({
        ...p,
        running: s.running,
        day: s.day,
        steps: s.steps,
        finished: s.finished,
        startedAt: s.running && s.started_ms_ago !== null ? Date.now() - s.started_ms_ago : null,
        stepStartedAt: s.running ? (p.stepStartedAt ?? Date.now()) : null,
      }));
      if (was && !s.running) settled.current();
    } catch (e) {
      setSt((p) => ({ ...p, running: false, error: toAppError(e) }));
    }
  }, []);
  useEffect(() => {
    void sync();
  }, [sync]);
  useEffect(() => {
    if (!st.running || attached.current) return;
    const id = setInterval(() => void sync(), 1000);
    return () => clearInterval(id);
  }, [st.running, sync]);

  const start = useCallback((force?: boolean) => {
    if (running.current) return;
    running.current = true;
    attached.current = true;
    const touched = new Set<string>();
    const end = (patch: Partial<State>) => {
      attached.current = false;
      setSt((p) => ({ ...p, running: false, cancelling: false, stepStartedAt: null, ...patch }));
    };
    const onEvent = (ev: RefreshEvent) => {
      switch (ev.type) {
        case "begin":
        case "done":
          touched.add(ev.step.name);
          setSt((p) => ({
            ...p,
            steps: upsert(p.steps, ev.step),
            stepStartedAt: ev.type === "begin" ? Date.now() : p.stepStartedAt,
          }));
          break;
        case "finished":
          end({ finished: ev, day: ev.run.day, steps: ev.run.steps });
          settled.current();
          break;
        case "could_not_start":
          end({ error: ev.error });
          void sync(); // nothing ran: the last recorded run is still the one to show
          break;
        case "died":
          end({ died: ev.message });
          settled.current();
          break;
      }
    };
    setSt((p) => ({ ...idle, day: p.day, running: true, startedAt: Date.now() }));
    refreshStart(onEvent, force)
      .then(refreshState)
      .then((s) => {
        // The four steps and the day, for rows no event has reached yet.
        if (attached.current)
          setSt((p) => ({ ...p, day: s.day, steps: s.steps.map((x) => (touched.has(x.name) ? (p.steps.find((y) => y.name === x.name) ?? x) : x)) }));
      })
      .catch((e: unknown) => {
        if (!attached.current) return;
        end({ error: toAppError(e) });
        void sync(); // "busy": a run this window did not start is in flight
      });
  }, [sync]);

  const cancel = useCallback(() => {
    setSt((p) => ({ ...p, cancelling: true }));
    refreshCancel().catch(() => setSt((p) => ({ ...p, cancelling: false })));
  }, []);

  return { ...st, start, cancel, sync: useCallback(() => void sync(), [sync]) };
}
