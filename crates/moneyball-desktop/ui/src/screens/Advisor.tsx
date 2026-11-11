import Markdown from "markdown-to-jsx";
import { Fragment, useCallback, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import {
  sessionCurrent,
  sessionNew,
  sessionOpen,
  sessionsList,
  toAppError,
  turnCancel,
  turnStart,
  type AppError,
  type SessionSnapshot,
  type TurnEvent,
  type ViewItem,
} from "../ipc";
import { ActionView, ErrorNote, Glyph, Loading, useAsync, useNow, when, type OnAction } from "../lib";

/** A line the app adds to the log itself: a failure, a dead worker, a save that missed. */
type Note = { type: "note"; text: string };
type Row = ViewItem | Note;

type Props = { hidden: boolean; newSession: number; onAction: OnAction };

export function Advisor({ hidden, newSession, onAction }: Props) {
  const sessions = useAsync(sessionsList, []);
  const [id, setId] = useState<string | null>(null);
  const [rows, setRows] = useState<Row[]>([]);
  const [live, setLive] = useState(""); // the answer being streamed
  const liveText = useRef(""); // the same, readable from an event handler
  const [active, setActive] = useState(false);
  const [polling, setPolling] = useState(false); // a turn runs with no channel to it (after a reload)
  const [since, setSince] = useState<number | null>(null);
  const [refusal, setRefusal] = useState<AppError | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [text, setText] = useState("");
  const log = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const now = useNow(active && since !== null);

  const show = useCallback((s: SessionSnapshot) => {
    setId(s.id);
    setRows(s.items);
    liveText.current = "";
    setLive("");
    setActive(s.turn_active);
    setPolling(s.turn_active);
    setSince(null);
    setRefusal(null);
    setLoadError(null);
    setLoaded(true);
  }, []);
  const { reload: reloadSessions } = sessions;

  useEffect(() => {
    sessionCurrent().then(show, (e: unknown) => setLoadError(toAppError(e)));
  }, [show]);

  // After a reload mid-turn there is nothing to subscribe to: poll the snapshot.
  useEffect(() => {
    if (!polling) return;
    const timer = setInterval(() => {
      sessionCurrent().then(
        (s) => {
          show(s);
          if (!s.turn_active) reloadSessions();
        },
        () => {},
      );
    }, 1000);
    return () => clearInterval(timer);
  }, [polling, show, reloadSessions]);

  const swap = useCallback(
    (to: Promise<SessionSnapshot>) => {
      to.then(
        (s) => {
          show(s);
          reloadSessions();
          input.current?.focus();
        },
        (e: unknown) => setRefusal(toAppError(e)),
      );
    },
    [show, reloadSessions],
  );
  useEffect(() => {
    if (newSession > 0) swap(sessionNew());
  }, [newSession, swap]);

  const onEvent = useCallback(
    (ev: TurnEvent) => {
      const end = () => {
        setActive(false);
        liveText.current = "";
        setLive("");
        reloadSessions();
      };
      switch (ev.type) {
        case "assistant_delta":
          setSince((s) => s ?? Date.now());
          liveText.current += ev.text;
          setLive(liveText.current);
          break;
        case "assistant_done":
          liveText.current = "";
          setLive("");
          setSince(null);
          setRows((r) => [...r, { type: "assistant", text: ev.text }]);
          break;
        case "tool_begin":
          setRows((r) => [...r, { type: "tool", call_id: ev.call_id, name: ev.name, args: ev.args, status: "pending", output: "" }]);
          break;
        case "tool_end":
          setRows((r) =>
            r.map((x) =>
              x.type === "tool" && x.call_id === ev.call_id ? { ...x, status: ev.ok ? "ok" : "failed", output: ev.output } : x,
            ),
          );
          break;
        case "item_done": // persistence is the shell's; the rows above already show it
          break;
        case "turn_complete":
          end();
          break;
        case "turn_aborted": {
          // Keep what had streamed, then the marker a reopened session would show.
          const cut = liveText.current;
          setRows((r) => [...r, ...(cut ? [{ type: "assistant" as const, text: cut }] : []), { type: "turn_interrupted" }]);
          end();
          break;
        }
        case "failed":
          setRows((r) => [...r, { type: "note", text: ev.error }]);
          end();
          break;
        case "died":
          setRows((r) => [...r, { type: "note", text: ev.message }]);
          end();
          break;
        case "save_failed":
          setRows((r) => [...r, { type: "note", text: ev.message }]);
          break;
      }
    },
    [reloadSessions],
  );

  const send = () => {
    const say = text.trim();
    if (!say || active) return;
    setRefusal(null);
    setText("");
    setRows((r) => [...r, { type: "user", text: say }]);
    setActive(true);
    setSince(null);
    const started = turnStart(say, onEvent);
    // A new session has no id until its first turn writes the file: learn it, so the list shows and marks it.
    if (id === null)
      started
        .then(sessionCurrent)
        .then((s) => {
          setId(s.id);
          reloadSessions();
        })
        .catch(() => {});
    started.catch((e: unknown) => {
      // Refused: nothing was recorded, so the message goes back to the composer.
      setRows((r) => {
        const i = r.findLastIndex((x) => x.type === "user" && x.text === say);
        return i < 0 ? r : r.filter((_, j) => j !== i);
      });
      setActive(false);
      setText((t) => t || say);
      setRefusal(toAppError(e));
    });
  };
  const stop = useCallback(() => void turnCancel().catch(() => {}), []);

  useEffect(() => {
    if (hidden || !active) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") stop();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hidden, active, stop]);

  useEffect(() => {
    if (!hidden) input.current?.focus();
  }, [hidden]);
  useEffect(() => {
    const el = log.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [rows, live, refusal, hidden]);

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    }
  };

  const tool = rows.findLast((r) => r.type === "tool" && r.status === "pending");
  const status = !active
    ? null
    : polling
      ? "Working on a turn that began before this window reloaded"
      : live
        ? `Answering${since !== null ? `, ${Math.max(0, Math.floor((now - since) / 1000))}s` : ""}`
        : tool?.type === "tool"
          ? `Running ${tool.name}`
          : "Thinking";

  return (
    <section className="screen" hidden={hidden} aria-label="Advisor">
      <header className="bar" data-tauri-drag-region>
        <h1>Advisor</h1>
        <span className="sub">Reads your data. Never changes your ads or your CRM.</span>
        <div className="tools">
          <button className="btn" onClick={() => swap(sessionNew())}>
            New Session <kbd>{"⌘"}N</kbd>
          </button>
        </div>
      </header>
      <div className="adv">
        <nav className="sess" aria-label="Sessions">
          {sessions.error && <p className="cap">{sessions.error.reason}</p>}
          {sessions.data?.length === 0 && <p className="cap">No saved sessions yet.</p>}
          {sessions.data?.map((s) => (
            <button key={s.id} aria-current={s.id === id ? "true" : undefined} onClick={() => s.id !== id && swap(sessionOpen(s.id))}>
              <b>{s.preview ?? "New session"}</b>
              <span>{when(s.modified_at)}</span>
            </button>
          ))}
        </nav>
        <div className="chat">
          <div className="log" ref={log}>
            {loadError ? <ErrorNote error={loadError} onAction={onAction} /> : !loaded && <Loading />}
            {loaded && rows.length === 0 && !live && <p className="cap">Ask about spend, leads, qualified and what to keep or kill. Answers come from your snapshots.</p>}
            {rows.map((r, i) => (
              <RowView key={i} row={r} active={active} />
            ))}
            {live && <Say text={live} live />}
            {refusal && (
              <div className="note" role="status">
                <span className="cap1">{refusal.reason}</span>
                {refusal.fix && (
                  <span className="inl">
                    <ActionView action={refusal.fix} onAction={onAction} />
                  </span>
                )}
              </div>
            )}
          </div>
          <div className="compose">
            <div className="row">
              <textarea
                ref={input}
                aria-label="Message"
                placeholder="Ask about your portfolio"
                rows={Math.min(6, text.split("\n").length)}
                value={text}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={onKeyDown}
              />
              {active ? (
                <button className="btn" onClick={stop}>
                  <i className="sq" />
                  Stop <kbd>esc</kbd>
                </button>
              ) : (
                <button className="btn primary" onClick={send} disabled={!text.trim() || !loaded}>
                  Send
                </button>
              )}
            </div>
            <div className="stat" role="status">
              {status && (
                <>
                  <Glyph kind="run" />
                  {status}
                </>
              )}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function RowView({ row, active }: { row: Row; active: boolean }) {
  switch (row.type) {
    case "user":
      return <div className="me">{row.text}</div>;
    case "assistant":
      return <Say text={row.text} />;
    case "tool":
      return <ToolRow row={row} active={active} />;
    case "turn_interrupted":
      return <p className="mark">Turn stopped</p>;
    case "bug_report_filed":
      return <p className="mark">Bug report filed</p>;
    case "note":
      return (
        <div className="note" role="status">
          <span className="cap1">{row.text}</span>
        </div>
      );
  }
}

/** Tool arguments as the model sent them, values only: "Disha Namma Mane, adset, 7". */
function argText(args: unknown): string {
  if (!args || typeof args !== "object") return "";
  return Object.values(args)
    .filter((v) => typeof v === "string" || typeof v === "number" || typeof v === "boolean")
    .join(", ");
}

function ToolRow({ row, active }: { row: Extract<ViewItem, { type: "tool" }>; active: boolean }) {
  const [open, setOpen] = useState(false);
  // Pending with no turn in flight is a call an interrupt cut short: it never got a result.
  const running = row.status === "pending" && active;
  const glyph = running ? "run" : row.status === "pending" ? "idle" : row.status === "ok" ? "ok" : "bad";
  const arg = argText(row.args);
  return (
    <>
      <button className="tool" aria-expanded={row.output ? open : undefined} aria-busy={running} onClick={() => setOpen((o) => !o)}>
        <Glyph kind={glyph} />
        <code>{row.name}</code>
        {arg && <span className="arg">{arg}</span>}
        {row.status !== "ok" && <span className="res">{running ? "running" : row.status === "pending" ? "stopped" : "failed"}</span>}
      </button>
      {open && row.output && <pre className="out">{row.output}</pre>}
    </>
  );
}

// Model output is untrusted: no raw HTML, links are shown but never
// navigated, images never load (the webview has no network anyway).
const Link = ({ children, href }: { children?: ReactNode; href?: string }) => (
  <span className="link" title={href}>
    {children}
  </span>
);
const MD = {
  disableParsingRawHTML: true,
  forceBlock: true,
  wrapper: Fragment, // blocks sit straight in .say, so its spacing rules reach them whether there is one or many
  overrides: { a: { component: Link }, img: { component: () => null }, iframe: { component: () => null } },
};

function Say({ text, live }: { text: string; live?: boolean }) {
  return (
    <div className={live ? "say live" : "say"}>
      <Markdown options={MD}>{text}</Markdown>
    </div>
  );
}
