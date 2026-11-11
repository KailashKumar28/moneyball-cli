import { useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { reportDays, reportFor, reportGenerate, reportOpenInBrowser, toAppError, type AppError, type ReportView } from "../ipc";
import { ErrorNote, Glyph, Loading, useAsync, type OnAction } from "../lib";

export type ReportHandle = { daily: () => void };
/** `onGenerated` tells the app the files on disk changed (it bumps `version`, so Today re-reads). */
type Props = { hidden: boolean; handle: Ref<ReportHandle>; version: number; onAction: OnAction; onGenerated: () => void };

export function ReportScreen({ hidden, handle, version, onAction, onGenerated }: Props) {
  const days = useAsync(reportDays, [version]);
  const [date, setDate] = useState<string>(); // undefined = the data day
  const [win, setWin] = useState(0); // which of the day's windows; the shell words them and gives their days
  const existing = useAsync(() => reportFor(date), [date, version]);
  // A report generated here, until a refresh rewrites the files (version moves on).
  const [made, setMade] = useState<{ version: number; view: ReportView } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const view = (made?.version === version ? made.view : null) ?? existing.data ?? null;
  const options = days.data ?? [];
  const shown = date ?? view?.date ?? options[0]?.date;
  const windows = options.find((d) => d.date === shown)?.windows ?? [];
  const chosen = windows[win];
  const current = useRef(version);
  useEffect(() => {
    current.current = version;
  });

  const generate = (args: { date?: string; days: number }) => {
    setBusy(true);
    setError(null);
    reportGenerate(args)
      .then(
        (view) => {
          // Kept through the version this generate itself bumps; a later refresh moves past it.
          setMade({ version: current.current + 1, view });
          onGenerated();
        },
        (e: unknown) => setError(toAppError(e)),
      )
      .finally(() => setBusy(false));
  };
  const run = () => {
    if (shown && chosen) generate({ date: shown, days: chosen.days });
  };
  // A "report" Action elsewhere in the app means the daily report for the data day, whatever was
  // last selected here: the selections go back to it, and the shell picks the day.
  const daily = () => {
    setDate(undefined);
    setWin(0);
    setMade(null);
    generate({ days: 1 });
  };
  useImperativeHandle(handle, () => ({ daily }));

  const pickDate = (d: string) => {
    setMade(null);
    setError(null);
    setDate(d);
  };
  const open = () => {
    if (view) reportOpenInBrowser(view.path).catch((e: unknown) => setError(toAppError(e)));
  };
  const problem = error ?? existing.error ?? days.error;

  return (
    <section className="screen" hidden={hidden} aria-label="Report">
      <header className="bar" data-tauri-drag-region>
        <h1>Report</h1>
        <div className="tools">
          <select className="sel" aria-label="Report day" value={shown ?? ""} onChange={(e) => pickDate(e.target.value)} disabled={options.length === 0}>
            {options.length === 0 && <option value="">No days</option>}
            {options.map((d) => (
              <option key={d.date} value={d.date}>
                {d.day}
              </option>
            ))}
          </select>
          <select className="sel" aria-label="Window" value={win} onChange={(e) => setWin(Number(e.target.value))} disabled={windows.length === 0}>
            {windows.map((w, i) => (
              <option key={w.label} value={i}>
                {w.label}
              </option>
            ))}
          </select>
          <button className="btn" onClick={run} disabled={busy || !chosen}>
            {busy && <Glyph kind="run" />}
            Generate
          </button>
          <button className="btn primary" onClick={open} disabled={!view}>
            Open in Browser
          </button>
        </div>
      </header>
      {problem && (
        <div className="over">
          <ErrorNote error={problem} onAction={onAction} />
        </div>
      )}
      <div className="body flush">
        {view ? (
          // sandbox with no tokens: no scripts, no navigation, no same-origin.
          <iframe sandbox="" srcDoc={view.html} title={`Report for ${view.day}`} />
        ) : (
          <div className="blank">
            {existing.loading || days.loading ? (
              <Loading />
            ) : (
              !problem && <p className="cap">{shown ? "No report is written for this day. Generate builds one from the snapshot." : "No snapshot yet, so there is no report to show."}</p>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
