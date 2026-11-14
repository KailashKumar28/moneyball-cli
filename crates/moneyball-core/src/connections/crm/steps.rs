//! The preset connect as plain steps, no Prompter: a preset's fields as
//! a form, save them, a live test pull as a typed result, save a stage
//! map. The terminal flow (crm.rs) is a loop over these; the Mac app
//! calls them from a form. A field's value never comes back out.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::config::AppConfig;
use crate::crm::{fetch as crm_fetch, presets, source, CANONICAL_STAGES};
use crate::error::{Error, Result};

/// The test pull's window, the refresh's: its crm.json replaces the
/// day's, so a shorter one would shrink what /brief and the report read.
pub const TEST_DAYS: u32 = 28;

/// A preset CRM as a form for this workspace.
#[derive(Debug, Clone, Serialize)]
pub struct PresetForm {
    pub id: &'static str,
    pub display: &'static str,
    pub fields: Vec<FormField>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormField {
    /// What `save_preset` takes the value under.
    pub key: &'static str,
    pub label: &'static str,
    /// Where to find the value.
    pub help: &'static str,
    /// Typed hidden.
    pub secret: bool,
    /// A value is stored for this workspace: an empty answer keeps it.
    pub stored: bool,
    /// The CRM refused the stored value on the last pull.
    pub refused: bool,
}

impl FormField {
    /// An empty value is fine only when one is stored.
    pub fn require(&self, value: &str) -> Result<()> {
        if value.is_empty() && !self.stored {
            return Err(Error::Config(format!("{} is required", self.label)));
        }
        Ok(())
    }
}

/// What `save_preset` did with crm.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecSaved {
    /// Written from the preset: a first connect, or a different CRM.
    Written,
    /// The same CRM's spec was there: kept, with its stage map.
    Kept,
}

/// A live test pull, in figures. `passed`: the check found no error and
/// the day's crm.json was written.
#[derive(Debug, Clone, Serialize)]
pub struct TestPull {
    pub name: String,
    pub tickets: usize,
    pub pages: u32,
    /// Organic/direct leads dropped for having no ad id.
    pub dropped_no_ad_id: usize,
    /// The check report, one line per finding ("-" info, "warn", "error").
    pub lines: Vec<String>,
    /// Stage names moneyball doesn't know, most frequent first.
    pub unknown_stages: Vec<String>,
    pub passed: bool,
}

/// CRM keys belong to a client: no workspace, nowhere to store them.
pub(super) fn workspace(cfg: &AppConfig) -> Result<&str> {
    cfg.workspace_id().ok_or_else(|| {
        Error::Config("no workspace here - CRM keys belong to a client's workspace".into())
    })
}

/// Every preset as a form for this workspace.
pub fn preset_forms(cfg: &AppConfig) -> Result<Vec<PresetForm>> {
    let ws = workspace(cfg)?;
    Ok(presets::catalog()
        .iter()
        .map(|c| form(cfg, ws, c))
        .collect())
}

pub(super) fn form(cfg: &AppConfig, ws: &str, c: &presets::Preset) -> PresetForm {
    // A key the CRM refused on the last pull: keeping it changes nothing.
    let refused = crate::crm::status::refused_on_last_pull(cfg).and_then(|step| step.fix);
    let fields = c
        .secrets
        .iter()
        .map(|s| {
            let name = c.secret_name(s);
            FormField {
                key: s.key,
                label: s.label,
                help: s.help,
                secret: true,
                stored: crate::secrets::load_crm_key(Some(ws), &name).is_some(),
                refused: refused.as_ref().is_some_and(|fix| fix.contains(&name)),
            }
        })
        .collect();
    PresetForm {
        id: c.id,
        display: c.display,
        fields,
    }
}

/// Store the preset's values (by field key; empty keeps a stored one)
/// for this workspace and save its spec. Every field is checked before
/// anything is stored. Reconnecting the same CRM keeps its spec and the
/// stage map mapped last time with it.
pub fn save_preset(
    cfg: &AppConfig,
    id: &str,
    values: &HashMap<String, String>,
) -> Result<SpecSaved> {
    let ws = workspace(cfg)?;
    let c = presets::catalog()
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Error::Config(format!("no CRM preset named {}", id)))?;
    let form = form(cfg, ws, &c);
    let value = |key: &str| values.get(key).map_or("", |v| v.trim());
    for f in &form.fields {
        f.require(value(f.key))?;
    }
    for s in c.secrets {
        let v = value(s.key);
        if !v.is_empty() {
            crate::secrets::store_crm_key(ws, &c.secret_name(s), v)?;
        }
    }
    let path = crm_fetch::spec_path(cfg);
    let same = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| source::parse(&raw).ok())
        .is_some_and(|spec| spec.name == c.id);
    if same {
        return Ok(SpecSaved::Kept);
    }
    save_spec(&path, c.render())?;
    Ok(SpecSaved::Written)
}

/// Pull the last `TEST_DAYS` with the saved spec; on a pass the day's
/// crm.json is written, as a refresh would.
pub fn test_pull(cfg: &AppConfig) -> Result<TestPull> {
    let r = crm_fetch::fetch_crm(cfg, TEST_DAYS)?;
    Ok(TestPull {
        name: r.name,
        tickets: r.tickets,
        pages: r.pages,
        dropped_no_ad_id: r.dropped_no_ad_id,
        lines: r.check.lines(),
        unknown_stages: r.check.unknown_stages,
        passed: r.path.is_some(),
    })
}

/// Map unknown stage names: each to one of `CANONICAL_STAGES`, or None
/// to keep it as is. True when the map changed - a confirming
/// `test_pull` is then due.
pub fn save_stage_map(cfg: &AppConfig, choices: &[(String, Option<String>)]) -> Result<bool> {
    let pairs: Vec<(String, String)> = choices
        .iter()
        .filter_map(|(stage, to)| Some((stage.clone(), to.clone()?)))
        .collect();
    if let Some((_, to)) = pairs
        .iter()
        .find(|(_, to)| !CANONICAL_STAGES.contains(&to.as_str()))
    {
        return Err(Error::Config(format!(
            "\"{}\" is not a stage moneyball knows - pick one of {}",
            to,
            CANONICAL_STAGES.join(", ")
        )));
    }
    if pairs.is_empty() {
        return Ok(false);
    }
    let path = crm_fetch::spec_path(cfg);
    let text = std::fs::read_to_string(&path)?;
    save_spec(&path, source::add_stage_mappings(&text, &pairs)?)?;
    Ok(true)
}

/// crm.toml, atomically: a half-written spec fails every pull after it.
pub(super) fn save_spec(path: &Path, body: String) -> Result<()> {
    let dir = path.parent().expect("crm.toml sits in .moneyball");
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("crm.toml");
    crate::secrets::write_private(dir, name, body).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_map_saves_the_mapped_names_and_only_known_stages() {
        let root = std::env::temp_dir().join(format!("mb-crm-stagemap-{}", std::process::id()));
        let cfg = AppConfig {
            data_root: root.clone(),
            date: None,
            workspace: None,
            agent: true,
        };
        std::fs::create_dir_all(cfg.mb_dir()).unwrap();
        let spec = "name = \"x\"\n[map]\nad_id = \"a\"\nstage = \"s\"\ndelivery = \"d\"\n";
        let path = crm_fetch::spec_path(&cfg);
        std::fs::write(&path, spec).unwrap();

        // Only keeps: nothing to save, the spec untouched.
        assert!(!save_stage_map(&cfg, &[("Cold".into(), None)]).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), spec);
        // A target that is not a stage is refused before anything is written.
        let e = save_stage_map(&cfg, &[("Hot".into(), Some("Warm".into()))]).unwrap_err();
        assert!(e.to_string().contains("\"Warm\" is not a stage"), "{}", e);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), spec);

        let choices = [
            ("Hot Lead".to_string(), Some("Contactable".to_string())),
            ("Cold".to_string(), None),
        ];
        assert!(save_stage_map(&cfg, &choices).unwrap());
        let saved = source::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved.map.stage_map,
            HashMap::from([("Hot Lead".to_string(), "Contactable".to_string())])
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_field_may_be_empty_only_when_a_value_is_stored() {
        let mut f = FormField {
            key: "token",
            label: "API token",
            help: "",
            secret: true,
            stored: false,
            refused: false,
        };
        assert_eq!(
            f.require("").unwrap_err().to_string(),
            "config: API token is required"
        );
        assert!(f.require("tok").is_ok());
        f.stored = true;
        assert!(f.require("").is_ok());
    }
}
