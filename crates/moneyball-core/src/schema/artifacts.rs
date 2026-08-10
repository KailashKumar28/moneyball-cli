//! Snapshot-side artifact contracts: creatives.json, leads.json,
//! crm_contacts.json. See mod.rs for the versioning policy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

// ---------- leads.json (snapshot artifact, written by /fetch) ----------

/// `snap/<date>/leads.json` - per-lead Meta records for the fetched
/// window. RAW PII (names/phones/emails) by explicit user decision
/// 2026-08-10 ("local raw is okay; revisit at Postgres time"): file is
/// written 0600 and NEVER syncs (docs/CLOUD_PLAN.md sync matrix).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LeadsFile {
    /// `moneyball.leads/<major>`.
    pub schema: String,
    pub fetched_at: String,
    pub rows: Vec<LeadRow>,
}

/// One Meta lead submission, as the lead-gen form captured it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct LeadRow {
    /// Meta leadgen id - joins to CRM tickets that carry it.
    pub lead_id: String,
    pub ad_id: String,
    /// RFC3339 with offset, as Meta returns it.
    #[serde(default)]
    pub created_time: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

// ------- crm_contacts.json (snapshot artifact, written by crm fetch) -------

/// Contacts of the ORGANIC/direct records the crm.json transform drops
/// (no ad id) - the re-inquiry check needs them. Same raw-PII policy
/// as leads.json.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CrmContactsFile {
    /// `moneyball.crm_contacts/<major>`.
    pub schema: String,
    pub fetched_at: String,
    pub rows: Vec<ContactRow>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ContactRow {
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}
