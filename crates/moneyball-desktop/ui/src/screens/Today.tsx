import type { Screen } from "../App";
import type { Today } from "../ipc";
import { ActionView, Glyph, Notice, Pending, stepGlyph, Strip, type Async, type OnAction } from "../lib";

type Props = { hidden: boolean; today: Async<Today>; onAction: OnAction; onRefresh: () => void; go: (s: Screen) => void };

export function TodayScreen({ hidden, today, onAction, onRefresh, go }: Props) {
  return (
    <section className="screen" hidden={hidden} aria-label="Today">
      <header className="bar" data-tauri-drag-region>
        <h1>Today</h1>
        <div className="tools">
          <button className="btn" onClick={onRefresh}>
            Refresh
          </button>
          <button className="btn primary" onClick={() => go("report")}>
            Open Report
          </button>
        </div>
      </header>
      <div className="body">
        <Pending state={today} onAction={onAction}>
          {(t) => <Body t={t} onAction={onAction} />}
        </Pending>
      </div>
    </section>
  );
}

function Body({ t, onAction }: { t: Today; onAction: OnAction }) {
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
      {day?.numbers && <Strip numbers={day.numbers} note={day.numbers_note} changes={day.versus?.changes} />}
      {day?.versus && <p className="cap">Changes are against {day.versus.day}.</p>}
      {day?.report_missing && (
        <div className="inl missing">
          <span>{day.report_missing.note}</span>
          <ActionView action={day.report_missing.next} onAction={onAction} />
        </div>
      )}
      {run && (
        <>
          <div className="h2">Data health</div>
          <table className="t">
            <thead>
              <tr>
                <th>Source</th>
                <th className="tx">Day</th>
                <th className="tx">State</th>
                <th className="tx">What came in</th>
              </tr>
            </thead>
            <tbody>
              {run.steps.map((s) => (
                <tr key={s.name}>
                  <td className="nm cap1">{s.label}</td>
                  {/* The run's day is only true of a step that brought data in. */}
                  <td className="tx">{s.status === "done" || s.status === "partial" ? run.day : ""}</td>
                  <td className="tx">
                    <span className="st">
                      <Glyph kind={stepGlyph(s.status)} />
                      {s.word}
                    </span>
                  </td>
                  <td className="tx">{s.detail}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {run.gaps && (
            <p className="cap">
              <Glyph kind="warn" /> <span className="cap1">{run.gaps}</span>
            </p>
          )}
        </>
      )}
    </>
  );
}
