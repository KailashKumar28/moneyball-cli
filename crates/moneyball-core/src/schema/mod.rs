//! Persisted-artifact contracts (docs/CLOUD_PLAN.md phase 1, slice A0).
//!
//! Every JSON artifact moneyball itself writes carries a
//! `schema: "moneyball.<artifact>/<major>"` field. These serde structs
//! are the source of truth; `tests/schema_contract.rs` generates JSON
//! Schemas from them into `docs/schemas/` and fails when the wire shape
//! changes without a conscious re-commit.
//!
//! Compatibility policy (binding): within a major, changes are
//! additive-only - new optional fields with serde defaults; readers
//! ignore unknown fields; meaning/type/requiredness of existing fields
//! never changes. Breaking change = new major in the `schema` string,
//! and loaders keep reading every prior major ever shipped.
//!
//! Legacy snapshot files (ads_daily.json, crm.json) are grandfathered
//! bare arrays ("v0") - documented, never retrofitted with envelopes.

mod artifacts;
mod report;

pub use artifacts::*;
pub use report::*;

pub const CREATIVES_SCHEMA: &str = "moneyball.creatives/1";
pub const CREATIVE_REPORT_SCHEMA: &str = "moneyball.creative_report/1";
pub const LEADS_SCHEMA: &str = "moneyball.leads/1";
pub const CRM_CONTACTS_SCHEMA: &str = "moneyball.crm_contacts/1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creative_row_defaults_tolerate_minimal_input() {
        // Forward-compat floor: an object with only ad_id parses, and
        // unknown fields are ignored.
        let r: CreativeRow = serde_json::from_str(r#"{"ad_id":"1","future_field":42}"#).unwrap();
        assert_eq!(r.ad_id, "1");
        assert!(!r.is_video && r.asset.is_none() && r.afs_video_ids.is_empty());
    }

    #[test]
    fn product_tag_round_trips_as_underscore_name() {
        let r = CreativeRow {
            ad_id: "1".into(),
            product: "NammaMane".into(),
            ..Default::default()
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["_product"], "NammaMane");
    }

    #[test]
    fn kpis_none_never_serializes_as_zero() {
        let k = Kpis::default();
        let v = serde_json::to_value(&k).unwrap();
        assert!(v["cost_per_qualified"].is_null());
        assert!(v["l_to_q_pct"].is_null());
    }
}
