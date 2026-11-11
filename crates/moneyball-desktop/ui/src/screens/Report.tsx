import { useImperativeHandle, useState, type Ref } from "react";
import { reportDays, reportFor, reportGenerate, reportOpenInBrowser, toAppError, type AppError, type ReportView } from "../ipc";
import { ErrorNote, Glyph, Loading, useAsync, type OnAction } from "../lib";

export type ReportHandle = { generate: () => void };
type Props = { hidden: boolean; handle: Ref<ReportHandle>; version: number; onAction: OnAction };
type Win = "1" | "7" | "mtd";

export function ReportScreen({ hidden, handle, version, onAction }: Props) {
  const days = useAsync(reportDays, [version]);
  const [date, setDate] = useState<string>(); // undefined = the data day
  const [win, setWin] = useState<Win>("1");
  const existing = useAsync(() => reportFor(date), [date, version]);
  // A report generated here, until a refresh rewrites the files (version moves on).
  const [made, setMade] = useState<{ version: number; view: ReportView } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const view = (made?.version === version ? made.view : null) ?? existing.data ?? null;
  const options = days.data ?? [];
  const shown = date ?? view?.date ?? options[0]?.date;

  const run = () => {
    const n = win === "mtd" ? (options.find((d) => d.date === shown)?.month_to_date ?? 1) : Number(win);
    setBusy(true);
    setError(null);
    reportGenerate({ date: shown, days: n })
      .then((view) => setMade({ version, view }), (e: unknown) => setError(toAppError(e)))
      .finally(() => setBusy(false));
  };
  // A "report" Action elsewhere in the app lands here and generates.
  useImperativeHandle(handle, () => ({ generate: run }));

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
          <select className="sel" aria-label="Window" value={win} onChange={(e) => setWin(e.target.value as Win)}>
            <option value="1">1 day</option>
            <option value="7">7 days</option>
            <option value="mtd">Month to date</option>
          </select>
          <button className="btn" onClick={run} disabled={busy || !shown}>
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
