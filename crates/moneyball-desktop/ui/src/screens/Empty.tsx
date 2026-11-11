import type { AppError } from "../ipc";
import { ErrorNote, Glyph, Term } from "../lib";

/** No workspace open: a first run, a cancelled picker, or a folder that would not open. */
export function Empty(props: { loading: boolean; error: AppError | null; onPick: () => void; back: { name: string; go: () => void } | null }) {
  return (
    <div className="empty">
      <div className="dragzone" data-tauri-drag-region />
      {props.loading ? (
        <p className="cap loading">
          <Glyph kind="run" /> Opening
        </p>
      ) : (
        <div>
          <h2>Open a workspace</h2>
          <p>Moneyball reads the workspace folder that setup created: its snapshots, CRM pulls and reports. It never changes your ads or your CRM.</p>
          {props.error && <ErrorNote error={props.error} onAction={() => {}} />}
          <div className="inl">
            <button className="btn primary" onClick={props.onPick}>
              Open Workspace... <kbd>{"⌘"}O</kbd>
            </button>
            {props.back && (
              <button className="btn" onClick={props.back.go}>
                Back to {props.back.name}
              </button>
            )}
          </div>
          {!props.error && (
            <div className="alt">
              <p>No workspace yet? Setup runs in the terminal for now.</p>
              <div className="inl">
                <Term command="moneyball" />
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
