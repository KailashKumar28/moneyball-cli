// The Add a Client sheet (plans/desktop/onboarding-plan.md Steps 1 and 2,
// plans/desktop/auth-plan.md): one modal, two pages. Connect Meta comes
// first only while no token is stored; a token Meta stops accepting sends
// the sheet back to it. Every sentence here about Meta, the folder or the
// accounts is the shell's.
import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import {
  clientAccounts,
  clientCreate,
  clientFolder,
  clientFolderPick,
  metaConnect,
  metaConnectCancel,
  onboardingState,
  toAppError,
  type AccountChoice,
  type AppError,
  type Folder,
  type MetaConnected,
  type Workspace,
} from "../ipc";
import { ErrorNote, Glyph, Loading, useAsync } from "../lib";

type Props = { onClose: () => void; onCreated: (w: Workspace) => void };

export function AddClient({ onClose, onCreated }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const st = useAsync(onboardingState, []);
  // Set once the state is in; true again when Meta stops accepting the token.
  const [withMeta, setWithMeta] = useState<boolean | null>(null);
  const [page, setPage] = useState<"meta" | "client" | null>(null);
  const [refused, setRefused] = useState<AppError | null>(null);
  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
  }, []);
  const hasMeta = withMeta ?? (st.data ? st.data.meta !== "complete" : null);
  const shown = page ?? (hasMeta === null ? null : hasMeta ? "meta" : "client");
  const head = useId();
  return (
    <dialog
      ref={dialog}
      className="sheet"
      aria-labelledby={head}
      onCancel={(e) => {
        e.preventDefault(); // Escape: the sheet closes through its owner, which unmounts it
        onClose();
      }}
    >
      {shown === null && (
        <div className="sbody">{st.error ? <ErrorNote error={st.error} onAction={() => {}} /> : <Loading />}</div>
      )}
      {shown === "meta" && <ConnectMeta head={head} refused={refused} onNext={() => setPage("client")} onCancel={onClose} />}
      {shown === "client" && (
        <ClientPage
          head={head}
          step={hasMeta === true}
          onBack={hasMeta ? () => setPage("meta") : null}
          onCancel={onClose}
          onRefused={(e) => {
            setRefused(e);
            setWithMeta(true);
            setPage("meta");
          }}
          onCreated={onCreated}
        />
      )}
    </dialog>
  );
}

/** Page one: Continue with Facebook, finished in the system browser; the shell waits for it. */
function ConnectMeta(props: { head: string; refused: AppError | null; onNext: () => void; onCancel: () => void }) {
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<AppError | null>(props.refused);
  const [ok, setOk] = useState<MetaConnected | null>(null);
  // Bumped by every start and stop: the answer to an earlier one is dropped.
  const turn = useRef(0);
  const pending = useRef(false);
  const settle = (mine: number, then: () => void) => {
    if (turn.current !== mine) return;
    pending.current = false;
    setWaiting(false);
    then();
  };
  const start = () => {
    const mine = ++turn.current;
    pending.current = true;
    setWaiting(true);
    setError(null);
    metaConnect().then(
      (m) => settle(mine, () => setOk(m)),
      (err: unknown) => settle(mine, () => setError(toAppError(err))),
    );
  };
  const stop = () => {
    turn.current++;
    pending.current = false;
    setWaiting(false);
    metaConnectCancel().catch(() => {});
  };
  // The sheet closed mid-wait (Escape): the shell stops waiting too.
  useEffect(
    () => () => {
      if (pending.current) metaConnectCancel().catch(() => {});
    },
    [],
  );
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (ok) props.onNext();
    else if (!waiting) start();
  };
  return (
    <form onSubmit={submit}>
      <header>
        <p className="lbl">Step 1 of 2</p>
        <h2 id={props.head}>Connect Meta</h2>
      </header>
      <div className="sbody">
        <p className="lead">Moneyball will be able to read the ads, and the leads, of the ad accounts and Pages you choose on Facebook.</p>
        {!waiting && !ok && (
          <button className="btn primary connect" type="submit" autoFocus>
            Continue with Facebook
          </button>
        )}
        {waiting && (
          <div className="waiting" role="status">
            <p className="inl">
              <Glyph kind="run" />
              <b>Finish in your browser</b>
            </p>
            <p className="lead">Facebook is open in your browser: choose the business, its ad accounts and Pages, then come back here.</p>
          </div>
        )}
        {error && (
          <p className="err" role="alert">
            {error.reason}
          </p>
        )}
        {ok && <Connected m={ok} />}
      </div>
      <footer>
        {/* While the browser is out, Cancel stops waiting for it; otherwise it closes the sheet. */}
        <button className="btn" type="button" onClick={waiting ? stop : props.onCancel}>
          Cancel
        </button>
        {ok && (
          <button className="btn primary" type="submit" autoFocus>
            Next
          </button>
        )}
      </footer>
    </form>
  );
}

function Connected({ m }: { m: MetaConnected }) {
  return (
    <div className="got" role="status">
      <dl>
        <dt>Connected as</dt>
        <dd>{m.name}</dd>
        <dt>Ad accounts</dt>
        <dd className="num">{m.accounts}</dd>
        <dt>Expiry</dt>
        <dd>{m.expiry}</dd>
        {m.read_only && (
          <>
            <dt>Access</dt>
            <dd>Read-only</dd>
          </>
        )}
      </dl>
      {m.warnings.map((w) => (
        <p className="note" key={w}>
          {w}
        </p>
      ))}
    </div>
  );
}

/** `value`, once it has stopped changing for `ms`. */
function useSettled(value: string, ms: number): string {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(id);
  }, [value, ms]);
  return settled;
}

type Pick = { on: boolean; name: string };

/** Page two: the name, its folder, and the ad accounts its products come from. */
function ClientPage(props: {
  head: string;
  step: boolean;
  onBack: (() => void) | null;
  onCancel: () => void;
  onRefused: (e: AppError) => void;
  onCreated: (w: Workspace) => void;
}) {
  const [name, setName] = useState("");
  const [chosen, setChosen] = useState<Folder | null>(null);
  // What the user changed, by account id; untouched rows follow the shell's `suggested`.
  const [picks, setPicks] = useState<Record<string, Pick>>({});
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const settled = useSettled(name, 400);
  const folder = useAsync(() => clientFolder(name), [name]);
  const accounts = useAsync(() => clientAccounts(settled), [settled]);
  const field = useId();

  const { onRefused } = props;
  useEffect(() => {
    // The contract's one branch on a kind: the token stopped working, so Connect Meta again.
    if (accounts.error?.kind === "meta_auth") onRefused(accounts.error);
  }, [accounts.error, onRefused]);

  const pickOf = (a: AccountChoice): Pick => picks[a.id] ?? { on: a.suggested, name: a.name };
  const change = (a: AccountChoice, to: Partial<Pick>) => {
    setPicks((p) => ({ ...p, [a.id]: { ...pickOf(a), ...to } }));
    setError(null);
  };
  const pickFolder = () => {
    clientFolderPick().then((f) => f && setChosen(f), (e: unknown) => setError(toAppError(e)));
  };
  const create = (e: FormEvent) => {
    e.preventDefault();
    const products = (accounts.data ?? [])
      .filter((a) => pickOf(a).on)
      .map((a) => ({ account_id: a.id, name: pickOf(a).name }));
    setCreating(true);
    setError(null);
    clientCreate({ name, folder: chosen?.path ?? null, products }).then(props.onCreated, (err: unknown) => {
      setError(toAppError(err));
      setCreating(false);
    });
  };
  const where = chosen ?? folder.data;
  return (
    <form onSubmit={create}>
      <header>
        {props.step && <p className="lbl">Step 2 of 2</p>}
        <h2 id={props.head}>Add a Client</h2>
      </header>
      <div className="sbody">
        <label className="field" htmlFor={field}>
          Client name
        </label>
        <input
          id={field}
          className="input wide"
          value={name}
          autoFocus
          spellCheck={false}
          onChange={(e) => {
            setName(e.target.value);
            setError(null);
          }}
        />
        <div className="inl where">
          <span className="lbl" title={where?.path}>
            {where?.display ?? ""}
          </span>
          <button className="btn small" type="button" onClick={pickFolder}>
            Change...
          </button>
        </div>
        {accounts.error && accounts.error.kind !== "meta_auth" && <ErrorNote error={accounts.error} onAction={() => {}} />}
        {!accounts.data && !accounts.error && <Loading />}
        {accounts.data && (
          <table className="t accts">
            <thead>
              <tr>
                <th className="tx">Ad account</th>
                <th className="tx">Product name</th>
                <th className="tx">Status</th>
              </tr>
            </thead>
            <tbody>
              {accounts.data.map((a) => {
                const p = pickOf(a);
                return (
                  <tr key={a.id}>
                    <td className="tx">
                      <label className="pick" title={`${a.name} (${a.id})`}>
                        <input type="checkbox" checked={p.on} onChange={(e) => change(a, { on: e.target.checked })} />
                        <span>{a.name}</span>
                      </label>
                    </td>
                    <td className="tx">
                      <input
                        className="input cell"
                        aria-label={`Product name for ${a.name}`}
                        value={p.name}
                        disabled={!p.on}
                        spellCheck={false}
                        onChange={(e) => change(a, { name: e.target.value })}
                      />
                    </td>
                    <td className="tx">
                      {a.status}
                      {a.used_by && <span className="tag">{a.used_by}</span>}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        {error && (
          <p className="err" role="alert">
            {error.reason}
          </p>
        )}
      </div>
      <footer>
        {props.onBack && (
          <button className="btn back" type="button" onClick={props.onBack}>
            Back
          </button>
        )}
        <button className="btn" type="button" onClick={props.onCancel}>
          Cancel
        </button>
        <button className="btn primary" type="submit" disabled={creating}>
          {creating && <Glyph kind="run" />}
          Create Client
        </button>
      </footer>
    </form>
  );
}
