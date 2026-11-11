import { brief, type BriefView } from "../ipc";
import { ErrorNote, Notice, num, pct, Pending, RS, rupees, useAsync, type OnAction } from "../lib";

type Props = { hidden: boolean; sub: string; version: number; onAction: OnAction };

export function BriefScreen({ hidden, sub, version, onAction }: Props) {
  const state = useAsync(brief, [version]);
  return (
    <section className="screen" hidden={hidden} aria-label="Brief">
      <header className="bar" data-tauri-drag-region>
        <h1>Brief</h1>
        <span className="sub">{sub}</span>
      </header>
      <div className="body">
        <Pending state={state} onAction={onAction}>
          {(v) => <Body v={v} onAction={onAction} />}
        </Pending>
      </div>
    </section>
  );
}

function Body({ v, onAction }: { v: BriefView; onAction: OnAction }) {
  if (v.fault) return <ErrorNote error={v.fault} onAction={onAction} />;
  if (!v.brief) return <p className="cap">No snapshot yet. Today shows what to do.</p>;
  const { rows, feasibility: f, has_crm: crm } = v.brief;
  return (
    <>
      {v.staleness && <Notice text={v.staleness} onAction={onAction} />}
      {v.crm_note && <Notice strong="CRM columns hidden." text={v.crm_note} onAction={onAction} />}
      <table className="t">
        <thead>
          <tr>
            <th>Product</th>
            <th>Spend/day</th>
            <th>Meta</th>
            {crm && <th>CRM</th>}
            {crm && <th>Qual.</th>}
            {crm && <th>Per day</th>}
            {crm && <th>{RS}/qual.</th>}
            {crm && <th>L to Q</th>}
            <th>Goal</th>
            {crm && <th>Gap</th>}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.product}>
              <td className="nm" title={r.product}>
                {r.product}
              </td>
              <td>{num(r.spend_per_day)}</td>
              <td>{num(r.m7d)}</td>
              {crm && <td>{num(r.l7d)}</td>}
              {crm && <td>{num(r.q7d)}</td>}
              {crm && <td>{num(r.q_per_day, 2)}</td>}
              {crm && <td>{num(r.rs_per_q)}</td>}
              {crm && <td>{pct(r.l_to_q)}</td>}
              <td>{num(r.goal)}</td>
              {crm && <td>{num(r.gap, 1)}</td>}
            </tr>
          ))}
        </tbody>
      </table>
      <p className="cap">
        {crm
          ? "Meta, CRM and Qual. are leads over the 7 days. Gap is qualified per day short of the goal."
          : "Meta is leads over the 7 days. Goal is qualified per day."}
      </p>
      {v.cpl_order && <p className="cap cap1">{v.cpl_order}</p>}
      <div className="feas">
        <div className="h2">Feasibility</div>
        {v.feasibility_line && <p className="line">{v.feasibility_line}</p>}
        <dl>
          {f.required_at_cur !== null && (
            <>
              <dt>Needed at today's cost</dt>
              <dd>{rupees(f.required_at_cur)} / day</dd>
            </>
          )}
          {f.required_at_best !== null && (
            <>
              <dt>Needed at the best cost{f.best_rpq !== null && `, ${rupees(f.best_rpq)}`}</dt>
              <dd>{rupees(f.required_at_best)} / day</dd>
            </>
          )}
          <dt>Open setup debt</dt>
          <dd>{f.open_debt.length}</dd>
        </dl>
        {f.open_debt.map((d) => (
          <p className="cap" key={d}>
            {d}
          </p>
        ))}
      </div>
    </>
  );
}
