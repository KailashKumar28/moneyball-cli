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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CREATIVES_SCHEMA: &str = "moneyball.creatives/1";
pub const CREATIVE_REPORT_SCHEMA: &str = "moneyball.creative_report/1";

// ---------- creatives.json (snapshot artifact, written by /fetch) ----------

/// `snap/<date>/creatives.json` - per-ad creative identity + asset refs.
/// Envelope object (new artifacts always get one); the snapshot loader
/// rule is: bare array => v0 rows, object => read `schema` + `rows`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreativesFile {
    /// Artifact identity, `moneyball.creatives/<major>`. Readers reject
    /// majors they don't know; unknown FIELDS are ignored everywhere.
    pub schema: String,
    /// When the fetch ran (RFC3339, UTC). Provenance, not a join key.
    pub fetched_at: String,
    pub rows: Vec<CreativeRow>,
}

/// One ad's creative facts. Facts only - the creative GROUPING key is
/// computed at report time (families/name-normalization are editorial
/// policy, not snapshot facts).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CreativeRow {
    // ---- identity (join keys) ----
    /// Meta ad id. Primary key within a snapshot.
    pub ad_id: String,
    #[serde(default)]
    pub adset_id: String,
    #[serde(default)]
    pub campaign_id: String,
    #[serde(default)]
    pub ad_name: String,
    /// Workspace product tag - same rule as ads_daily rows.
    #[serde(rename = "_product", default)]
    pub product: String,

    // ---- creative identity facts (grouping inputs) ----
    /// Meta's content hash for image ads; None for video.
    #[serde(default)]
    pub image_hash: Option<String>,
    /// Top-level or video_data video id; None for image ads.
    #[serde(default)]
    pub video_id: Option<String>,
    /// Dynamic (asset_feed_spec) video ids, sorted. Empty if none.
    #[serde(default)]
    pub afs_video_ids: Vec<String>,
    /// CDN asset filename from image_url, query stripped. Legacy
    /// identity fallback only (python creative_key()).
    #[serde(default)]
    pub image_basename: Option<String>,
    /// Derived: video_id or afs_video_ids non-empty.
    #[serde(default)]
    pub is_video: bool,

    // ---- display / status ----
    /// Meta effective_status verbatim (ACTIVE, PAUSED, ...).
    #[serde(default)]
    pub status: Option<String>,
    /// Creative created time as Meta returns it (RFC3339 with offset).
    #[serde(default)]
    pub created_time: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub cta: Option<String>,
    /// IG/FB permalink when Meta exposes one.
    #[serde(default)]
    pub permalink: Option<String>,

    // ---- asset refs ----
    /// Full-res URL AT FETCH TIME. Signatures expire within days; the
    /// read path never dereferences this - the asset cache is truth.
    #[serde(default)]
    pub image_url: Option<String>,
    /// None if the download failed (report renders a placeholder).
    #[serde(default)]
    pub asset: Option<AssetRef>,
}

/// A cached creative image, content-addressed under
/// `history/assets/creatives/<hh>/<sha256>.<ext>`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssetRef {
    /// Content hash of the cached bytes = cache filename stem.
    pub sha256: String,
    pub content_type: String,
    pub bytes: u64,
}

// ---------- report.json (moneyball report output) ----------

/// `reports/<date>/creative-report.json` - THE product artifact. The
/// HTML renderer, a bot text-summarizer, and the future service's
/// Postgres ingester consume exactly this; the renderer never touches
/// snapshots. Every array element carries its own natural key so rows
/// upsert losslessly: (workspace_id, report_date) > product > group_key
/// > targeting/date.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreativeReport {
    /// `moneyball.creative_report/<major>`.
    pub schema: String,
    /// Stable workspace UUID (minted once into config.json). With
    /// report_date + schema major: the idempotent-upsert key.
    pub workspace_id: String,
    /// Snapshot date the report was computed FROM (YYYY-MM-DD).
    pub report_date: String,
    pub window: ReportWindow,
    /// RFC3339 UTC generation timestamp.
    pub generated_at: String,
    pub source: ReportSource,
    /// Portfolio KPIs - a bot renders exactly this block as the headline.
    pub portfolio: Kpis,
    pub products: Vec<ProductSection>,
}

/// Days of data the report aggregates (inclusive, YYYY-MM-DD).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReportWindow {
    pub since: String,
    pub until: String,
}

/// Provenance for audit / re-derivation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReportSource {
    pub snapshot_date: String,
    pub crm_present: bool,
    /// Schema string of the creatives.json consumed; None when the
    /// snapshot predates creatives capture (cards fall back to
    /// one-ad-per-card grouping).
    #[serde(default)]
    pub creatives_schema: Option<String>,
}

/// Delivery + funnel roll-up. Numbers are NUMBERS here - Meta's
/// string-typing is a snapshot-layer quirk, the aggregate is typed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Kpis {
    pub spend: f64,
    pub impressions: u64,
    pub clicks: u64,
    pub funnel: FunnelCounts,
    /// None when qualified == 0 - never 0-as-sentinel.
    #[serde(default)]
    pub cost_per_qualified: Option<f64>,
    /// None when l_leads == 0.
    #[serde(default)]
    pub l_to_q_pct: Option<f64>,
    /// CRM tickets with no joinable ad_id (includes the intentional
    /// "Stattic Ad" rows) - counted, never silently dropped.
    #[serde(default)]
    pub unattributed_l_leads: u64,
}

/// CRM-joined funnel counts (Meta m_leads + the four CRM milestones).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct FunnelCounts {
    pub m_leads: u64,
    pub l_leads: u64,
    pub qualified: u64,
    pub visit: u64,
    pub booking: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProductSection {
    pub product: String,
    pub kpis: Kpis,
    /// Sorted python-style: booking desc, then visit, qualified,
    /// l_leads, m_leads.
    pub creatives: Vec<CreativeCard>,
}

/// One creative (a group of ads sharing an image/video), the report's
/// unit of comparison.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreativeCard {
    /// Grouping key with its kind prefixed, e.g. `img:<image_hash>`,
    /// `vid:<video_id>`, `family:<slug>`, `ad:<ad_id>`. PK within
    /// (workspace, report_date, product).
    pub group_key: String,
    pub group_kind: GroupKind,
    /// Most-common ad_name in the group.
    pub display_name: String,
    pub ad_ids: Vec<String>,
    pub campaigns: Vec<String>,
    pub is_video: bool,
    pub status: CardStatus,
    /// Earliest creative created date in the group (YYYY-MM-DD).
    #[serde(default)]
    pub created: Option<String>,
    /// None => renderers show a placeholder.
    #[serde(default)]
    pub image: Option<ImageRef>,
    pub delivery: Delivery,
    /// The seven canonical stages, ordered, ALWAYS all present -
    /// renderers iterate this array, never reconstruct it.
    pub funnel: Vec<FunnelStage>,
    #[serde(default)]
    pub targetings: Vec<TargetingBreakdown>,
    /// Daily buckets across the trailing window, oldest first.
    #[serde(default)]
    pub trend: Vec<TrendBucket>,
}

/// How the group was identified (precedence: family > video_name >
/// video_id > image_hash > image_basename > ad_id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    Family,
    VideoName,
    VideoId,
    ImageHash,
    ImageBasename,
    AdId,
}

/// live | learn | stop with a display label (learning-stage detail is
/// a v2 additive field).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CardStatus {
    pub code: StatusCode,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusCode {
    Live,
    Learn,
    Stop,
}

/// Pointer into the content-addressed asset cache; `path` is relative
/// to `<workspace>/.moneyball/history/`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImageRef {
    pub sha256: String,
    pub path: String,
}

/// Meta delivery metrics for a card or targeting row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Delivery {
    pub spend: f64,
    pub impressions: u64,
    #[serde(default)]
    pub reach: u64,
    pub clicks: u64,
    pub m_leads: u64,
}

/// One step of the canonical funnel. Stage names are fixed:
/// Impressions, Clicks, M-Leads, L-Leads, Qualified, Visit, Booking.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FunnelStage {
    pub stage: String,
    pub count: u64,
}

/// Per-targeting (= base adset name, campaign suffix stripped) split.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TargetingBreakdown {
    pub targeting: String,
    pub delivery: Delivery,
    pub crm: TargetingCrm,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TargetingCrm {
    pub l_leads: u64,
    pub qualified: u64,
    pub visit: u64,
    pub booking: u64,
}

/// One day of a card's trend series.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TrendBucket {
    /// YYYY-MM-DD.
    pub date: String,
    pub impressions: u64,
    pub clicks: u64,
    pub m_leads: u64,
    pub l_leads: u64,
    pub qualified: u64,
    pub visit: u64,
    pub booking: u64,
}

/// The canonical funnel stage names, in order. Report building uses
/// this; renderers use the array embedded in each card.
pub const FUNNEL_STAGES: [&str; 7] = [
    "Impressions",
    "Clicks",
    "M-Leads",
    "L-Leads",
    "Qualified",
    "Visit",
    "Booking",
];

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
