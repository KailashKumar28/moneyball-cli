import { useState } from "react";
import type { Screen } from "../App";
import { funnel, type By, type FunnelView } from "../ipc";
import { Notice, num, Pending, RS, useAsync, type OnAction } from "../lib";

type Props = { hidden: boolean; products: string[]; version: number; onAction: OnAction; go: (s: Screen) => void };
type Days = 7 | 14 | 28;
const DAYS: Days[] = [7, 14, 28];
const LEVELS: [By, string][] = [
  ["campaign", "Campaign"],
  ["adset", "Ad set"],
  ["ad", "Ad"],
];

export function FunnelScreen({ hidden, products, version, onAction, go }: Props) {
  const [product, setProduct] = useState(products[0] ?? "");
  const [days, setDays] = useState<Days>(7);
  const [by, setBy] = useState<By>("adset");
  const state = useAsync(() => funnel({ product, days, by }), [product, days, by, version]);
  const level = LEVELS.find(([id]) => id === by)?.[1] ?? "";
  return (
    <section className="screen" hidden={hidden} aria-label="Funnel">
      <header className="bar" data-tauri-drag-region>
        <h1>Funnel</h1>
        <div className="tools">
          <select className="sel" aria-label="Product" value={product} onChange={(e) => setProduct(e.target.value)}>
            {products.map((p) => (
              <option key={p}>{p}</option>
            ))}
          </select>
          <div className="seg" role="radiogroup" aria-label="Days">
            {DAYS.map((d) => (
              <Seg key={d} name="days" label={`${d}d`} checked={days === d} pick={() => setDays(d)} />
            ))}
          </div>
          <div className="seg" role="radiogroup" aria-label="Level">
            {LEVELS.map(([id, label]) => (
              <Seg key={id} name="level" label={label} checked={by === id} pick={() => setBy(id)} />
            ))}
          </div>
        </div>
      </header>
      <div className="body">
        {products.length === 0 ? (
          <p className="cap">This workspace has no products configured.</p>
        ) : (
          <Pending state={state} onAction={onAction}>
            {(v) => <Body v={v} level={level} onAction={onAction} toToday={() => go("today")} />}
          </Pending>
        )}
      </div>
    </section>
  );
}

function Seg(props: { name: string; label: string; checked: boolean; pick: () => void }) {
  const id = `seg-${props.name}-${props.label}`;
  return (
    <>
      <input type="radio" name={props.name} id={id} checked={props.checked} onChange={props.pick} />
      <label htmlFor={id}>{props.label}</label>
    </>
  );
}

/** Four shapes that read without color: outline + dot, solid + cross, bare ring, dashed + "?". */
function Verdict({ text }: { text: string }) {
  const shape = text === "keep" ? "keep" : text === "kill" ? "kill" : text.toLowerCase() === "no crm" ? "nocrm" : "wait";
  return <span className={`v ${shape}`}>{text}</span>;
}

function Body({ v, level, onAction, toToday }: { v: FunnelView; level: string; onAction: OnAction; toToday: () => void }) {
  const crm = v.has_crm;
  return (
    <>
      {!crm && (
        <Notice strong="Verdicts withheld." text="This snapshot has no CRM data: qualified is unknown, not zero. Fix it on" onAction={onAction}>
          {" "}
          <button className="link" onClick={toToday}>
            Today
          </button>
          .
        </Notice>
      )}
      {v.rows.length === 0 ? (
        <p className="cap">Nothing ran for {v.product} in this window.</p>
      ) : (
        <table className="t">
          <thead>
            <tr>
              <th>{level}</th>
              <th>Spend</th>
              <th>Meta</th>
              <th>Per lead</th>
              {crm && <th>CRM</th>}
              {crm && <th>Qual.</th>}
              {crm && <th>{RS}/qual.</th>}
              <th className="tx">Verdict</th>
            </tr>
          </thead>
          <tbody>
            {v.rows.map((r) => (
              <tr key={r.id}>
                <td className="nm" title={r.name}>
                  {r.name}
                </td>
                <td>{num(r.spend)}</td>
                <td>{num(r.m)}</td>
                <td>{num(r.cpl)}</td>
                {crm && <td>{num(r.l)}</td>}
                {crm && <td>{num(r.q)}</td>}
                {crm && <td>{num(r.rs_per_q)}</td>}
                <td className="tx">
                  <Verdict text={r.verdict} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {v.cpl_ranking && <p className="cap cap1">{v.cpl_ranking}</p>}
      {v.notes.map((n) => (
        <p className="cap" key={n}>
          {n}
        </p>
      ))}
    </>
  );
}
