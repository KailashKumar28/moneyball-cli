import { useState } from "react";
import type { Screen } from "../App";
import { reportOpenInBrowser, toAppError, type AppError, type Today } from "../ipc";
import { ActionView, ErrorNote, Glyph, Notice, Pending, type Async, type OnAction } from "../lib";

type Props = { hidden: boolean; today: Async<Today>; onAction: OnAction; onRefresh: () => void; go: (s: Screen) => void };

export function TodayScreen({ hidden, today, onAction, onRefresh, go }: Props) {
  const [problem, setProblem] = useState<AppError | null>(null);
  const report = today.data?.day?.report ?? null;
  const open = () => {
    if (report) reportOpenInBrowser(report.path).then(() => setProblem(null), (e: unknown) => setProblem(toAppError(e)));
  };
  return (
    <section className="screen" hidden={hidden} aria-label="Today">
      <header className="bar" data-tauri-drag-region>
        <h1>Today</h1>
        <div className="tools">
          <button className="btn" onClick={onRefresh}>
            Refresh
          </button>
          <button className="btn primary" onClick={open} disabled={!report}>
            Open in Browser
          </button>
        </div>
      </header>
      {problem && (
        <div className="over">
          <ErrorNote error={problem} onAction={onAction} />
        </div>
      )}
      <div className="body today">
        <Pending state={today} onAction={onAction}>
          {(t) => <Body t={t} onAction={onAction} go={go} />}
        </Pending>
      </div>
    </section>
  );
}

function Body({ t, onAction, go }: { t: Today; onAction: OnAction; go: (s: Screen) => void }) {
  const { day, headline, last_run: run } = t;
  const kind = t.health.state.kind;
  const steps = t.next ? [...t.fixes, t.next] : t.fixes;
  return (
    <>
      <div className="day">
        <h2 className="cap1">{day?.day ?? t.health_label}</h2>
        {day && (
          <span className="badge">
            <Glyph kind={kind === "fresh" ? "dot" : kind === "failed" ? "bad" : "warn"} />
            <span className="cap1">{t.health_label}</span>
          </span>
        )}
        {day?.refreshed && <span className="when">Refreshed {day.refreshed}</span>}
      </div>
      <div className="ruler" />
      {(headline || steps.length > 0 || t.details.length > 0) && (
        <Notice strong={headline?.text} text={headline?.rest} details={t.details} steps={steps} onAction={onAction} />
      )}
      {/* The run's steps in full are on Refresh; here only what it missed. */}
      {run?.gaps && (
        <p className="cap gaps">
          <Glyph kind="warn" /> <span className="cap1">{run.gaps}</span>{" "}
          <button className="link" onClick={() => go("refresh")}>
            Details
          </button>
        </p>
      )}
      {day?.report && (
        // sandbox with no tokens: no scripts, no navigation, no same-origin.
        <iframe className="doc" sandbox="" srcDoc={day.report.html} title={`Report for ${day.report.day}`} />
      )}
      {day?.report_missing && (
        <div className="inl missing">
          <span>{day.report_missing.note}</span>
          <ActionView action={day.report_missing.next} onAction={onAction} />
        </div>
      )}
    </>
  );
}
