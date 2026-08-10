//! The report.json (CreativeReport) contract. See mod.rs for the
//! versioning policy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// The M-Leads -> L-Leads gap explained (None when leads.json is
    /// absent from the snapshot). Additive v1 field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segmentation: Option<Segmentation>,
}

/// Window Meta leads split by fate (python SEG_KEYS):
/// captured + reinquiry + duplicate + invalid + uncaptured == total.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Segmentation {
    pub total: u64,
    /// Became an L-Lead (lead_id present in CRM tickets).
    pub captured: u64,
    /// Contact already exists in the CRM under another campaign.
    pub reinquiry: u64,
    /// Same contact submitted again (CRM folds it).
    pub duplicate: u64,
    /// Phone not a valid mobile - CRM rejects.
    pub invalid: u64,
    /// Valid, unique, new - but not in the CRM: a genuine sync gap.
    pub uncaptured: u64,
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
