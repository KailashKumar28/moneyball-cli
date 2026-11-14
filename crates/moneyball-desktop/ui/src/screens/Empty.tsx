import { onboardingState, type AppError } from "../ipc";
import { ErrorNote, Glyph, useAsync } from "../lib";

type Props = {
  loading: boolean;
  error: AppError | null;
  onAdd: () => void;
  onPick: () => void;
  back: { name: string; go: () => void } | null;
};

/** No client open: a first launch, a cancelled picker, or a folder that would not open. */
export function Empty(props: Props) {
  const st = useAsync(onboardingState, []);
  const known = (st.data?.clients ?? 0) > 0;
  return (
    <div className="empty">
      <div className="dragzone" data-tauri-drag-region />
      {props.loading ? (
        <p className="cap loading">
          <Glyph kind="run" /> Opening
        </p>
      ) : (
        <div>
          <h2>{known ? "Open a client" : "Add your first client"}</h2>
          <p>Moneyball pulls each client's Meta ads and CRM leads every morning and shows what is working. It only reads: it never changes your ads or your CRM.</p>
          {props.error && <ErrorNote error={props.error} onAction={() => {}} />}
          <div className="inl">
            <button className="btn primary" onClick={props.onAdd}>
              Add a Client
            </button>
            <button className="btn" onClick={props.onPick}>
              Open a Folder... <kbd>{"⌘"}O</kbd>
            </button>
            {props.back && (
              <button className="btn" onClick={props.back.go}>
                Back to {props.back.name}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
