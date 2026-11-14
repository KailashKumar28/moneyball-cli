// Shared hooks, formatters and the small components every screen uses.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { copyText, toAppError, type Action, type AppError, type StepView, type Tint } from "./ipc";

// ---------- async ----------

export type Async<T> = { data: T | undefined; error: AppError | null; loading: boolean; reload: () => void };

/** Runs `fn` on mount and whenever `deps` change. Keeps the last data while reloading. */
export function useAsync<T>(fn: () => Promise<T>, deps: (string | number | undefined)[]): Async<T> {
  const [tick, setTick] = useState(0);
  const key = [tick, ...deps].join("\u0000");
  const [state, setState] = useState<{ key: string | null; data: T | undefined; error: AppError | null }>({
    key: null,
    data: undefined,
    error: null,
  });
  const latest = useRef(fn);
  useEffect(() => {
    latest.current = fn;
  });
  useEffect(() => {
    let live = true;
    latest.current().then(
      (data) => live && setState({ key, data, error: null }),
      (e: unknown) => live && setState({ key, data: undefined, error: toAppError(e) }),
    );
    return () => {
      live = false;
    };
  }, [key]);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  return { data: state.data, error: state.error, loading: state.key !== key, reload };
}

/** Date.now(), re-read every second while `active`. */
export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const tick = () => setNow(Date.now());
    const first = setTimeout(tick, 0);
    const id = setInterval(tick, 1000);
    return () => {
      clearTimeout(first);
      clearInterval(id);
    };
  }, [active]);
  return now;
}

// ---------- formatting (layout only: never arithmetic on figures) ----------

const int = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });
/** null is an empty cell, never 0. */
export const num = (v: number | null, digits = 0) =>
  v === null ? "" : digits ? v.toFixed(digits) : int.format(v);
/** Money is worded the way core words it in every sentence it sends, so one screen never shows two styles. */
export const RS = "Rs.";
export const rupees = (v: number | null) => (v === null ? "" : RS + int.format(v));
export const pct = (v: number | null) => (v === null ? "" : v.toFixed(1) + "%");

/** Elapsed wall time as 00:52. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/** A session's time: "Today 09:14", "Yesterday 17:53", "28 Sep". */
export function when(iso: string, now = new Date()): string {
  const t = new Date(iso);
  if (Number.isNaN(t.getTime())) return "";
  const hm = `${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (t.getTime() >= midnight) return `Today ${hm}`;
  if (t.getTime() >= midnight - 86_400_000) return `Yesterday ${hm}`;
  return `${t.getDate()} ${MONTHS[t.getMonth()]}`;
}

// ---------- components ----------

export type GlyphKind = "ok" | "bad" | "warn" | "idle" | "dot" | "run";
export const Glyph = ({ kind }: { kind: GlyphKind }) => <i className={`g ${kind}`} aria-hidden="true" />;

const STEP_GLYPH: Record<StepView["status"], GlyphKind> = {
  done: "ok",
  partial: "warn",
  failed: "bad",
  skipped: "warn",
  waiting: "idle",
  running: "run",
};
export const stepGlyph = (status: StepView["status"]): GlyphKind => STEP_GLYPH[status] ?? "warn";

/** What a step brought in, and the shell's note under it. */
export const StepDetail = ({ step }: { step: StepView }) => (
  <>
    {step.detail}
    {step.note && <span className="tag">{step.note}</span>}
  </>
);

export const TINT_GLYPH: Record<Tint, GlyphKind | null> = { plain: null, good: "ok", warn: "warn", bad: "bad" };

/** "In Terminal" + the command + Copy. The app never runs it. */
export function Term({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    copyText(command).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      },
      () => {},
    );
  };
  return (
    <>
      <span className="lbl">In Terminal</span>
      <span className="term">
        <code>{command}</code>
        <button onClick={copy}>{copied ? "Copied" : "Copy"}</button>
      </span>
    </>
  );
}

export type OnAction = (a: Action) => void;

/** One Action: a terminal command to copy, or a button that starts the work. */
export function ActionView({ action, onAction }: { action: Action; onAction: OnAction }) {
  return (
    <>
      {action.kind === "terminal" ? (
        <Term command={action.command} />
      ) : (
        <button className="btn" onClick={() => onAction(action)}>
          {action.label}
        </button>
      )}
      {action.does && <span className="why">{action.does}</span>}
    </>
  );
}

/** A problem sentence and its numbered fix steps. */
export function Notice(props: { strong?: string; text?: string; details?: string[]; steps?: Action[]; onAction: OnAction }) {
  const steps = props.steps ?? [];
  return (
    <div className="notice" role="status">
      <p>
        {props.strong && <b>{props.strong}</b>} {props.text}
      </p>
      {props.details?.map((d) => (
        <p className="why" key={d}>
          {d}
        </p>
      ))}
      {steps.length > 0 && (
        <ol>
          {steps.map((a, i) => (
            <li key={i}>
              {steps.length > 1 && <span className="n">{i + 1}</span>}
              <ActionView action={a} onAction={props.onAction} />
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

export const ErrorNote = ({ error, onAction }: { error: AppError; onAction: OnAction }) => (
  <Notice text={error.reason} steps={error.fix ? [error.fix] : []} onAction={onAction} />
);

export const Loading = () => (
  <p className="cap loading">
    <Glyph kind="run" /> Loading
  </p>
);

/** Loading, then the error with its fix, then the data. */
export function Pending<T>({ state, onAction, children }: { state: Async<T>; onAction: OnAction; children: (data: T) => ReactNode }) {
  if (state.error) return <ErrorNote error={state.error} onAction={onAction} />;
  if (state.data === undefined) return <Loading />;
  return <>{children(state.data)}</>;
}

/**
 * The four refresh steps. `elapsed` is the ticking time of the step in flight.
 * When the run is not `live`, a step still marked running or waiting never
 * finished (the run died or did not start): a bare ring, no spinner, no word.
 */
export function Steps(props: { steps: StepView[]; live: boolean; elapsed?: string; resultHead: string }) {
  const { steps, live, elapsed, resultHead } = props;
  const open = (s: StepView) => s.status === "running" || s.status === "waiting";
  return (
    <table className="t steps">
      <thead>
        <tr>
          <th></th>
          <th className="tx">Step</th>
          <th className="tx">State</th>
          <th className="tx">{resultHead}</th>
          <th>Time</th>
        </tr>
      </thead>
      <tbody>
        {steps.map((s, i) => (
          <tr key={s.name} className={s.status === "waiting" || (open(s) && !live) ? "wait" : undefined}>
            <td>{i + 1}</td>
            <td className="nm cap1">{s.label}</td>
            <td className="tx">
              <span className="st">
                <Glyph kind={open(s) && !live ? "idle" : stepGlyph(s.status)} />
                {open(s) && !live ? "" : s.word}
              </span>
            </td>
            <td className="tx wrap">
              <StepDetail step={s} />
            </td>
            <td>{s.status === "running" ? (live ? elapsed : "") : s.time}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
