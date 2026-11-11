//! A funnel row as it crosses a process boundary (a GUI's IPC). A row
//! computed from a snapshot with no CRM data has `kill: None` and zeros
//! for every CRM count; serialized, those counts are null - unknown, not
//! zero. Field names are the Rust ones.

use serde::{Serialize, Serializer};

use super::FunnelRow;

#[derive(Serialize)]
struct Row<'a> {
    id: &'a str,
    name: &'a str,
    spend: u64,
    m: u64,
    l: Option<u64>,
    q: Option<u64>,
    v: Option<u64>,
    b: Option<u64>,
    cpl: Option<u64>,
    rs_per_q: Option<u64>,
    l_to_q: Option<f64>,
    kill_mult: f64,
    kill: Option<bool>,
    sufficient: bool,
    immature: Option<bool>,
    learning: &'a str,
}

impl Serialize for FunnelRow {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // `kill` is None exactly when the snapshot had no CRM data.
        let crm = self.kill.is_some();
        Row {
            id: &self.id,
            name: &self.name,
            spend: self.spend,
            m: self.m,
            l: crm.then_some(self.l),
            q: crm.then_some(self.q),
            v: crm.then_some(self.v),
            b: crm.then_some(self.b),
            cpl: self.cpl,
            rs_per_q: self.rs_per_q.filter(|_| crm),
            l_to_q: self.l_to_q.filter(|_| crm),
            kill_mult: self.kill_mult,
            kill: self.kill,
            sufficient: self.sufficient,
            immature: crm.then_some(self.immature),
            learning: &self.learning,
        }
        .serialize(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_crm_data_is_null_never_zero() {
        let row = FunnelRow {
            name: "Set One".into(),
            spend: 9000,
            m: 4,
            ..Default::default()
        };
        let v = serde_json::to_value(&row).unwrap();
        for key in ["l", "q", "v", "b", "rs_per_q", "l_to_q", "kill", "immature"] {
            assert!(v[key].is_null(), "{} = {}", key, v[key]);
        }
        assert_eq!(v["spend"], 9000);
        assert_eq!(v["m"], 4);

        let with_crm = FunnelRow {
            kill: Some(false),
            ..row
        };
        let v = serde_json::to_value(&with_crm).unwrap();
        assert_eq!(v["q"], 0, "with CRM data a zero is a real zero");
        assert_eq!(v["kill"], false);
        assert_eq!(v["immature"], false);
    }

    #[test]
    fn by_serializes_as_its_name() {
        for by in [super::super::By::Campaign, super::super::By::Adset] {
            assert_eq!(serde_json::to_value(by).unwrap(), by.name());
        }
    }
}
