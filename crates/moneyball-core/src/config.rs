//! Configuration loader.
//!
//! Resolution: CLI flags > env > <data-root>/moneyball/config.json
//! > ~/.moneyball/config.json > built-in defaults.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const GLOBAL_CONFIG: &str = ".moneyball/config.json"; // relative to $HOME

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub name: String,
    pub ad_account: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrmConfig {
    #[serde(default = "default_crm_name")]
    pub name: String,
    #[serde(default = "default_stages")]
    pub stages: Vec<String>,
    #[serde(default = "default_qualified_stage")]
    pub qualified_stage: String,
    #[serde(default)]
    pub join_keys: CrmJoinKeys,
    #[serde(default)]
    pub bucket_by_delivery: bool,
}

fn default_crm_name() -> String { "custom".into() }
fn default_stages() -> Vec<String> {
    vec!["Lost".into(), "NonContactable".into(), "Contactable".into(),
         "Visit".into(), "Booking".into()]
}
fn default_qualified_stage() -> String { "Contactable".into() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrmJoinKeys {
    #[serde(default = "default_ad_id_path")]
    pub ad_id: String,
    #[serde(default = "default_adset_id_path")]
    pub adset_id: String,
}

fn default_ad_id_path() -> String { "adId.adId".into() }
fn default_adset_id_path() -> String { "adsetId.adsetId".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub products: Vec<Product>,
    #[serde(default)]
    pub goals: std::collections::HashMap<String, f64>,
    #[serde(default = "default_target_rpq")]
    pub target_rs_per_q: f64,
    #[serde(default)]
    pub crm: CrmConfig,
}

fn default_target_rpq() -> f64 { 2500.0 }

impl WorkspaceConfig {
    /// Load from `<data_root>/moneyball/config.json`.
    pub fn load(data_root: &Path) -> Result<Self> {
        let path = data_root.join("moneyball").join("config.json");
        if !path.is_file() {
            return Err(Error::NoWorkspaceConfig { path: path.display().to_string() });
        }
        let raw = std::fs::read_to_string(&path)?;
        let cfg: WorkspaceConfig = serde_json::from_str(&raw)
            .map_err(|e| Error::Config(format!("{}: {}", path.display(), e)))?;
        Ok(cfg)
    }

    pub fn product_names(&self) -> Vec<&str> {
        self.products.iter().map(|p| p.name.as_str()).collect()
    }

    pub fn ad_accounts(&self) -> std::collections::HashMap<&str, &str> {
        self.products.iter().map(|p| (p.name.as_str(), p.ad_account.as_str())).collect()
    }

    pub fn goal_for(&self, product: &str) -> f64 {
        self.goals.get(product).copied().unwrap_or(10.0)
    }

    /// Persist to `<data_root>/moneyball/config.json` (creates the dir if missing).
    pub fn save(&self, data_root: &Path) -> Result<()> {
        let path = data_root.join("moneyball").join("config.json");
        std::fs::create_dir_all(path.parent().unwrap())?;
        let pretty = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("serialize: {}", e)))?;
        std::fs::write(&path, pretty)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_root: PathBuf,
    pub date: Option<String>,
    pub workspace: Option<WorkspaceConfig>,
    pub agent: bool,
}

impl AppConfig {
    /// Resolve from CLI args + env + filesystem. Errors if no workspace config.
    pub fn resolve(data_root: Option<&str>, date: Option<&str>) -> Result<Self> {
        let root = resolve_data_root(data_root)?;
        let workspace = WorkspaceConfig::load(&root)?;
        let agent = std::env::var("MB_AGENT").map(|v| !v.is_empty()).unwrap_or(false);
        Ok(Self { data_root: root, date: date.map(String::from), workspace: Some(workspace), agent })
    }

    /// Resolve data-root without requiring a workspace config. Returns
    /// `workspace: None` if no config.json exists - the TUI uses this to
    /// detect first-run and show a setup wizard instead of erroring.
    pub fn resolve_optional(data_root: Option<&str>, date: Option<&str>) -> Self {
        let root = resolve_data_root(data_root).unwrap_or_else(|_| {
            std::env::current_dir().unwrap_or_default().join("moneyball-data")
        });
        let workspace = WorkspaceConfig::load(&root).ok();
        let agent = std::env::var("MB_AGENT").map(|v| !v.is_empty()).unwrap_or(false);
        Self { data_root: root, date: date.map(String::from), workspace, agent }
    }

    pub fn has_workspace(&self) -> bool { self.workspace.is_some() }

    pub fn snap_dir(&self) -> PathBuf {
        self.data_root.join("moneyball").join("history").join("snap")
    }

    pub fn snap_for(&self, date: Option<&str>) -> Result<PathBuf> {
        let snap = self.snap_dir();
        if !snap.is_dir() {
            return Err(Error::NoSnapshot { date: "<any>".into(), snap_root: snap.display().to_string() });
        }
        if let Some(d) = date {
            let p = snap.join(d);
            if !p.is_dir() {
                return Err(Error::NoSnapshot { date: d.into(), snap_root: snap.display().to_string() });
            }
            return Ok(p);
        }
        // latest
        let mut latest: Option<PathBuf> = None;
        for entry in std::fs::read_dir(&snap)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.len() == 10 && name.chars().nth(4) == Some('-') {
                        latest = Some(match latest {
                            Some(prev) if prev > p => prev,
                            _ => p,
                        });
                    }
                }
            }
        }
        latest.ok_or_else(|| Error::NoSnapshot { date: "<latest>".into(), snap_root: snap.display().to_string() })
    }

    pub fn state_dir(&self) -> PathBuf { self.data_root.join("moneyball").join("state") }
    pub fn history_dir(&self) -> PathBuf { self.data_root.join("moneyball").join("history") }
    pub fn runs_dir(&self) -> PathBuf { self.data_root.join("moneyball").join("runs") }

    pub fn mb_dir(&self) -> PathBuf { self.data_root.join("moneyball") }
}

fn resolve_data_root(cli_arg: Option<&str>) -> Result<PathBuf> {
    if let Some(s) = cli_arg {
        return Ok(PathBuf::from(s).canonicalize().unwrap_or_else(|_| PathBuf::from(s)));
    }
    // $MONEYBALL_DATA_ROOT env
    if let Ok(s) = std::env::var("MONEYBALL_DATA_ROOT") {
        return Ok(PathBuf::from(&s).canonicalize().unwrap_or_else(|_| PathBuf::from(s)));
    }
    // ~/.moneyball/config.json data_root
    if let Some(home) = dirs_home() {
        let gc = home.join(GLOBAL_CONFIG);
        if gc.is_file() {
            if let Ok(raw) = std::fs::read_to_string(&gc) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(dr) = v.get("data_root").and_then(|x| x.as_str()) {
                        return Ok(PathBuf::from(dr).canonicalize().unwrap_or_else(|_| PathBuf::from(dr)));
                    }
                }
            }
        }
    }
    // Walk up from CWD looking for a sibling `fin_campaign_analysis/` or `data/`.
    if let Ok(cwd) = std::env::current_dir() {
        let mut p: Option<&std::path::Path> = Some(&cwd);
        while let Some(dir) = p {
            for name in &["fin_campaign_analysis", "data"] {
                let cand = dir.join(name);
                if cand.is_dir() {
                    return Ok(cand);
                }
            }
            p = dir.parent();
        }
    }
    // Well-known home paths.
    if let Some(home) = dirs_home() {
        for rel in ["DEV/fin_campaign_analysis", "fin_campaign_analysis"] {
            let cand = home.join(rel);
            if cand.is_dir() {
                return Ok(cand);
            }
        }
    }
    // Last resort: default to a workspace directory next to where the user
    // is right now. They can edit during setup if they want different.
    let cwd = std::env::current_dir().unwrap_or_default();
    Ok(cwd.join("moneyball-data"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}