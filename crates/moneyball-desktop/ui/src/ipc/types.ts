// The IPC contract (plans/desktop/IPC.md), typed by hand.
// "core" types mirror the moneyball-core wire structs named beside each one.
// CRM-derived fields are nullable: null means "no CRM data", never zero.

export type Tint = "plain" | "good" | "warn" | "bad";

export type Action =
  | { kind: "refresh"; label: string; does: string; force: boolean }
  | { kind: "report"; label: string; does: string }
  | { kind: "terminal"; command: string; does: string };

export type AppError = { kind: string; reason: string; fix: Action | null };

export type Workspace = { name: string; path: string; display_path: string; products: string[] };

export type StepView = {
  name: "meta" | "assets" | "crm" | "report";
  label: string;
  // core's StepStatus names, plus the UI-only waiting and running
  status: "done" | "partial" | "failed" | "skipped" | "waiting" | "running";
  word: string;
  detail: string;
  time: string | null;
  reused: boolean;
  fix: Action | null;
};

export type RunView = {
  date: string;
  day: string;
  trigger: "manual" | "schedule";
  outcome: string;
  gaps: string;
  steps: StepView[];
};

// ---- core: schema/run.rs ----
export type RunStep = {
  name: string;
  status: "done" | "partial" | "failed" | "skipped";
  ms: number;
  counts: Record<string, number>;
  error_kind?: string;
  message?: string;
  fix?: string;
  reused?: boolean;
};
export type RunRecord = {
  schema: string;
  workspace_id: string;
  date: string;
  trigger: "schedule" | "manual";
  started_at: string;
  finished_at: string;
  outcome: "running" | "ok" | "degraded" | "failed";
  steps: RunStep[];
  artifacts: Record<string, string>;
};

// ---- core: health.rs ----
export type HealthState =
  | { kind: "failed" | "no_data" | "pinned" | "stale" | "crm_missing" | "fresh" }
  | { kind: "partial"; have: number; of: number };
export type Health = {
  today: string;
  snap_date: string | null;
  crm_snap_date: string | null;
  crm_connected: boolean;
  run: RunRecord | null;
  refreshed_at: string | null;
  failed_today: RunRecord | null;
  newest_snap_date: string | null;
  state: HealthState;
};

// `source` is a contract change (see NOTES.md): it colours the source dot.
export type DayNumber = { label: string; value: string; source?: "meta" | "crm" };

export type Today = {
  health: Health;
  health_label: string;
  footer: [string, string];
  headline: { tint: Tint; text: string; rest: string } | null;
  details: string[];
  fixes: Action[];
  next: Action | null;
  day: {
    day: string;
    refreshed: string | null;
    numbers: DayNumber[] | null;
    numbers_note: string | null;
    versus: { day: string; changes: { label: string; text: string; tint: Tint }[] } | null;
    report_missing: { note: string; next: Action } | null;
  } | null;
  last_run: RunView | null;
};

// ---- core: brief/wire.rs ----
export type BriefRow = {
  product: string;
  spend_per_day: number;
  m7d: number;
  cpl: number | null;
  l7d: number | null;
  q7d: number | null;
  q_per_day: number | null;
  rs_per_q: number | null;
  l_to_q: number | null; // percent, one decimal
  goal: number;
  gap: number | null;
  trend: string;
};
export type Feasibility = {
  tot_q_per_day: number | null;
  tot_spend_per_day: number;
  tot_goal_per_day: number;
  cur_rpq: number | null;
  required_at_cur: number | null;
  required_at_best: number | null;
  best_rpq: number | null;
  open_debt: string[];
};
export type ProductRowsAndFeasibility = {
  rows: BriefRow[];
  feasibility: Feasibility;
  snapshot_date: string;
  has_crm: boolean;
};

export type BriefView = {
  brief: ProductRowsAndFeasibility | null;
  crm_note: string | null;
  fault: AppError | null;
  staleness: string | null;
  cpl_order: string | null;
  feasibility_line: string | null;
};

// ---- core: funnel/wire.rs ----
export type FunnelRow = {
  id: string;
  name: string;
  spend: number;
  m: number;
  l: number | null;
  q: number | null;
  v: number | null;
  b: number | null;
  cpl: number | null;
  rs_per_q: number | null;
  l_to_q: number | null;
  kill_mult: number;
  kill: boolean | null;
  sufficient: boolean;
  immature: boolean | null;
  learning: string;
};
export type By = "campaign" | "adset" | "ad";
export type FunnelView = {
  product: string;
  has_crm: boolean;
  rows: (FunnelRow & { verdict: string })[];
  cpl_ranking: string | null;
  notes: string[];
};

export type RefreshState = { running: boolean; day: string; steps: StepView[]; started_ms_ago: number | null };

export type RefreshFinished = {
  type: "finished";
  run: RunView;
  time: string;
  verdict: { tint: Tint; headline: string; rest: string; next: Action | null } | null;
  fixes: Action[];
  rerun: Action | null;
  numbers: DayNumber[] | null;
  numbers_note: string | null;
  report_path: string | null;
};
export type RefreshEvent =
  | { type: "begin"; step: StepView }
  | { type: "done"; step: StepView }
  | RefreshFinished
  | { type: "could_not_start"; error: AppError }
  | { type: "died"; message: string };

export type ReportView = { date: string; day: string; days: number; path: string; html: string; summary: string };

// ---- core: session/summary.rs ----
export type SessionSummary = {
  id: string;
  started_at: string;
  modified_at: string;
  item_count: number;
  preview: string | null;
};

// ---- core: agent/view.rs ----
export type ToolStatus = "pending" | "ok" | "failed";
export type ViewItem =
  | { type: "user"; text: string }
  | { type: "assistant"; text: string }
  | { type: "tool"; call_id: string; name: string; args: unknown; status: ToolStatus; output: string }
  | { type: "turn_interrupted" }
  | { type: "bug_report_filed" };

export type SessionSnapshot = { id: string | null; items: ViewItem[]; turn_active: boolean };

// ---- core: agent.rs ----
export type Item =
  | { type: "user"; text: string }
  | { type: "assistant"; text: string }
  | { type: "tool_call"; call_id: string; name: string; args: unknown }
  | { type: "tool_output"; call_id: string; output: string; is_error?: boolean };
export type Ev =
  | { type: "assistant_delta"; text: string }
  | { type: "assistant_done"; text: string }
  | { type: "tool_begin"; call_id: string; name: string; args: unknown }
  | { type: "tool_end"; call_id: string; output: string; ok: boolean }
  | { type: "item_done"; item: Item }
  | { type: "turn_complete"; ms: number; provider: string; usage: { input: number; output: number } }
  | { type: "turn_aborted" }
  | { type: "failed"; error: string };

export type TurnEvent = Ev | { type: "died"; message: string } | { type: "save_failed"; message: string };
