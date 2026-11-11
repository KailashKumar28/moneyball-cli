//! The brief as it crosses a process boundary (a GUI's IPC). The Rust
//! structs keep CRM-derived numbers as plain zeros when the snapshot has
//! no CRM data and say so in `has_crm`; serialized, those numbers are
//! null - unknown, not zero (the rule brief_json set for MB_AGENT).
//!
//! That is why `ProductRow` and `Feasibility` are not Serialize on their
//! own: only the whole `ProductRowsAndFeasibility` knows `has_crm`, so
//! this is the one way a row gets out, and it cannot get out as zeros.
//! Field names are the Rust ones.

use serde::{Serialize, Serializer};

use super::{Feasibility, ProductRow, ProductRowsAndFeasibility};

#[derive(Serialize)]
struct Brief<'a> {
    rows: Vec<Row<'a>>,
    feasibility: Feas<'a>,
    snapshot_date: &'a str,
    has_crm: bool,
}

#[derive(Serialize)]
struct Row<'a> {
    product: &'a str,
    spend_per_day: u64,
    m7d: u64,
    cpl: Option<u64>,
    l7d: Option<u64>,
    q7d: Option<u64>,
    q_per_day: Option<f64>,
    rs_per_q: Option<u64>,
    l_to_q: Option<f64>,
    goal: f64,
    gap: Option<f64>,
    trend: &'a str,
}

#[derive(Serialize)]
struct Feas<'a> {
    tot_q_per_day: Option<f64>,
    tot_spend_per_day: u64,
    tot_goal_per_day: f64,
    cur_rpq: Option<u64>,
    required_at_cur: Option<u64>,
    required_at_best: Option<u64>,
    best_rpq: Option<u64>,
    open_debt: &'a [String],
}

fn row(p: &ProductRow, crm: bool) -> Row<'_> {
    Row {
        product: &p.product,
        spend_per_day: p.spend_per_day,
        m7d: p.m7d,
        cpl: p.cpl,
        l7d: crm.then_some(p.l7d),
        q7d: crm.then_some(p.q7d),
        q_per_day: crm.then_some(p.q_per_day),
        rs_per_q: p.rs_per_q.filter(|_| crm),
        l_to_q: p.l_to_q.filter(|_| crm),
        goal: p.goal,
        gap: crm.then_some(p.gap),
        trend: &p.trend,
    }
}

fn feasibility(f: &Feasibility, crm: bool) -> Feas<'_> {
    Feas {
        tot_q_per_day: crm.then_some(f.tot_q_per_day),
        tot_spend_per_day: f.tot_spend_per_day,
        tot_goal_per_day: f.tot_goal_per_day,
        cur_rpq: crm.then_some(f.cur_rpq),
        required_at_cur: f.required_at_cur.filter(|_| crm),
        required_at_best: f.required_at_best.filter(|_| crm),
        best_rpq: f.best_rpq.filter(|_| crm),
        open_debt: &f.open_debt,
    }
}

impl Serialize for ProductRowsAndFeasibility {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Brief {
            rows: self.rows.iter().map(|p| row(p, self.has_crm)).collect(),
            feasibility: feasibility(&self.feasibility, self.has_crm),
            snapshot_date: &self.snapshot_date,
            has_crm: self.has_crm,
        }
        .serialize(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(has_crm: bool) -> ProductRowsAndFeasibility {
        ProductRowsAndFeasibility {
            rows: vec![ProductRow {
                product: "P".into(),
                spend_per_day: 900,
                m7d: 12,
                cpl: Some(525),
                l7d: 0,
                q7d: 0,
                q_per_day: 0.0,
                rs_per_q: None,
                l_to_q: None,
                goal: 3.0,
                gap: 3.0,
                trend: "-".into(),
            }],
            feasibility: Feasibility {
                tot_q_per_day: 0.0,
                tot_spend_per_day: 900,
                tot_goal_per_day: 3.0,
                cur_rpq: 0,
                required_at_cur: None,
                required_at_best: None,
                best_rpq: None,
                open_debt: vec!["pixel".into()],
            },
            snapshot_date: "2026-11-10".into(),
            has_crm,
        }
    }

    #[test]
    fn no_crm_data_is_null_never_zero() {
        let v = serde_json::to_value(brief(false)).unwrap();
        let (r, f) = (&v["rows"][0], &v["feasibility"]);
        for key in ["l7d", "q7d", "q_per_day", "gap", "rs_per_q", "l_to_q"] {
            assert!(r[key].is_null(), "rows.{} = {}", key, r[key]);
        }
        for key in ["tot_q_per_day", "cur_rpq", "required_at_cur", "best_rpq"] {
            assert!(f[key].is_null(), "feasibility.{} = {}", key, f[key]);
        }
        // What Meta alone knows stays.
        assert_eq!(r["spend_per_day"], 900);
        assert_eq!(r["m7d"], 12);
        assert_eq!(f["open_debt"][0], "pixel");
        assert_eq!(v["has_crm"], false);
    }

    #[test]
    fn with_crm_data_a_zero_is_a_real_zero() {
        let v = serde_json::to_value(brief(true)).unwrap();
        assert_eq!(v["rows"][0]["q7d"], 0);
        assert_eq!(v["rows"][0]["gap"], 3.0);
        assert_eq!(v["feasibility"]["cur_rpq"], 0);
    }
}
