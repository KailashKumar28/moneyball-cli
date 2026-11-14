//! The clients the Mac app knows: every workspace it has opened or
//! created, kept as the `clients` key of `~/.moneyball/config.json` beside
//! `last_workspace`. The CLI and the TUI never read it, so where a shell
//! lands never changes.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::{home_dir, AppConfig, DOT_DIR};
use crate::error::{Error, Result};
use crate::workspace::{self, canonical};

const KEY: &str = "clients";

/// Every known client, opened, in the order first seen. A folder that no
/// longer holds a workspace is skipped, not forgotten: a disk that is not
/// mounted today may be tomorrow.
pub fn list() -> Vec<AppConfig> {
    home_dir()
        .map(|h| list_in(&h.join(DOT_DIR)))
        .unwrap_or_default()
}

/// Add the workspace at `dir` to the list (once).
pub fn add(dir: &Path) -> Result<()> {
    add_in(&global()?, dir).map(drop)
}

/// Take the workspace at `dir` off the list; its folder is not touched.
pub fn remove(dir: &Path) -> Result<()> {
    remove_in(&global()?, dir)
}

/// A client's name: its brand ("Fincity"), else its folder's name.
pub fn name(cfg: &AppConfig) -> String {
    let brand = cfg.workspace.as_ref().and_then(|w| w.brand.clone());
    brand.unwrap_or_else(|| {
        cfg.data_root.file_name().map_or_else(
            || cfg.data_root.display().to_string(),
            |n| n.to_string_lossy().into(),
        )
    })
}

fn global() -> Result<PathBuf> {
    home_dir()
        .map(|h| h.join(DOT_DIR))
        .ok_or_else(|| Error::Config("no home directory to keep the client list in".into()))
}

fn paths(global: &Path) -> Vec<String> {
    paths_of(workspace::read_global(global).unwrap_or_default().get(KEY))
}

pub(super) fn list_in(global: &Path) -> Vec<AppConfig> {
    paths(global)
        .iter()
        .filter_map(|p| workspace::open(Path::new(p), None).ok())
        .collect()
}

/// True when `dir` was not on the list before.
pub(super) fn add_in(global: &Path, dir: &Path) -> Result<bool> {
    let dir = workspace::path_key(dir)?;
    let mut added = false;
    workspace::edit_global(global, |keys| {
        let mut list = paths_of(keys.get(KEY));
        if !list.contains(&dir) {
            list.push(dir);
            added = true;
        }
        keys.insert(KEY.into(), list.into());
    })?;
    Ok(added)
}

pub(super) fn remove_in(global: &Path, dir: &Path) -> Result<()> {
    let dir = canonical(dir);
    workspace::edit_global(global, |keys| {
        let mut list = paths_of(keys.get(KEY));
        list.retain(|p| canonical(Path::new(p)) != dir);
        keys.insert(KEY.into(), list.into());
    })
}

fn paths_of(v: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(list)) = v else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|p| Some(p.as_str()?.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mb-clients-{}-{}", name, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        canonical(&dir)
    }

    fn client(at: &Path, brand: Option<&str>) -> PathBuf {
        crate::config::update(at, |w| {
            w.brand = brand.map(String::from);
            w.products = vec![crate::config::Product {
                name: "P".into(),
                ad_account: "111111".into(),
            }];
        })
        .unwrap();
        at.to_path_buf()
    }

    #[test]
    fn add_keeps_one_entry_and_every_other_key() {
        let root = tmp("add");
        let global = root.join(".moneyball");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(
            global.join("config.json"),
            r#"{"data_root": "/x", "last_workspace": "/y"}"#,
        )
        .unwrap();
        let a = client(&root.join("Fincity"), Some("Fincity"));
        assert!(add_in(&global, &a).unwrap());
        assert!(!add_in(&global, &a).unwrap(), "added once");
        let known = list_in(&global);
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].data_root, a);
        assert_eq!(name(&known[0]), "Fincity");
        let keys = workspace::read_global(&global).unwrap();
        assert_eq!(keys["data_root"], "/x");
        assert_eq!(keys["last_workspace"], "/y");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_skips_a_folder_that_is_gone_and_remove_takes_one_off() {
        let root = tmp("list");
        let global = root.join(".moneyball");
        let a = client(&root.join("a"), None);
        let b = client(&root.join("b"), None);
        add_in(&global, &a).unwrap();
        add_in(&global, &b).unwrap();
        std::fs::remove_dir_all(&a).unwrap();
        let known = list_in(&global);
        assert_eq!(known.len(), 1);
        assert_eq!(name(&known[0]), "b", "the folder's name without a brand");
        assert_eq!(paths(&global).len(), 2, "skipped, not forgotten");
        remove_in(&global, &b).unwrap();
        assert!(list_in(&global).is_empty());
        assert_eq!(paths(&global).len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }
}
