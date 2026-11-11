import type { Screen } from "../App";
import { clock, ErrorNote, Notice, Steps, useNow, type OnAction } from "../lib";
import type { Refresh } from "../useRefresh";

type Props = { hidden: boolean; refresh: Refresh; onAction: OnAction; go: (s: Screen) => void };

export function RefreshScreen({ hidden, refresh: r, onAction, go }: Props) {
  const now = useNow(r.running);
  const f = r.running ? null : r.finished;
  const title = r.running ? `Refreshing ${r.day}` : f ? `Refreshed ${f.run.day}` : r.day;
  const aside = r.running
    ? r.startedAt !== null && `${clock(now - r.startedAt)} so far`
    : f && [f.time, f.run.gaps].filter(Boolean).join(", ");
  const fixes = f ? [...f.fixes, ...(f.verdict?.next ? [f.verdict.next] : []), ...(f.rerun ? [f.rerun] : [])] : [];
  return (
    <section className="screen" hidden={hidden} aria-label="Refresh">
      <header className="bar" data-tauri-drag-region>
        <h1>Refresh</h1>
        <div className="tools">
          {r.running ? (
            <>
              <span className="lbl">{r.cancelling ? "Stopping after the current step" : "Stops after the current step"}</span>
              <button className="btn" onClick={r.cancel} disabled={r.cancelling}>
                <i className="sq" />
                Cancel
              </button>
            </>
          ) : (
            <button className="btn" onClick={() => r.start()}>
              {f ? "Refresh Again" : "Refresh"}
            </button>
          )}
        </div>
      </header>
      <div className="body">
        {r.error && <ErrorNote error={r.error} onAction={onAction} />}
        {r.died && <Notice text={r.died} onAction={onAction} />}
        {(title || aside) && (
          <div className="runhead">
            <h2>{title}</h2>
            <span>{aside}</span>
          </div>
        )}
        {r.steps.length > 0 ? (
          <Steps
            steps={r.steps}
            resultHead={r.running ? "Progress" : "Result"}
            elapsed={r.stepStartedAt !== null ? clock(now - r.stepStartedAt) : ""}
          />
        ) : (
          !r.running && !r.error && <p className="cap">No refresh has run in this workspace yet.</p>
        )}
        {r.running && <p className="cap">Brief, Funnel and the advisor keep working on the last good data while this runs.</p>}
        {f && (
          <div className="sum">
            {(f.verdict || fixes.length > 0) && (
              <Notice strong={f.verdict?.headline} text={f.verdict?.rest} steps={fixes} onAction={onAction} />
            )}
            {f.numbers && (
              <p>
                <b>{f.run.day}:</b>{" "}
                {f.numbers.map((n, i) => (
                  <span key={n.label}>
                    {i > 0 && ", "}
                    {n.label} <span className="num">{n.value}</span>
                  </span>
                ))}
                {f.numbers_note && <span className="gold">. {f.numbers_note}</span>}
              </p>
            )}
            <div className="acts">
              {f.report_path && (
                <button className="btn primary" onClick={() => go("report")}>
                  Open Report
                </button>
              )}
              <button className="btn" onClick={() => go("brief")}>
                Brief
              </button>
              <button className="btn" onClick={() => go("funnel")}>
                Funnel
              </button>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
