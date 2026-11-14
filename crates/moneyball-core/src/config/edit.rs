//! Writing the workspace config: `update` is the one way it is written,
//! and the workspace id it leaves with. Split from config.rs (size cap).

use std::path::Path;

use super::{migrate_legacy_layout, WorkspaceConfig, DOT_DIR};
use crate::error::{Error, Result};

/// Change the workspace config at `data_root` and write it - the one way
/// it is written. Under a lock beside it and from what is on disk now
/// (codex's ConfigEditsBuilder, pi's settings-manager), so the app and the
/// TUI on one client never undo each other's change with a stale copy. A
/// missing config starts empty; either way it leaves with a workspace id,
/// and whatever `change` does not touch is kept (the id, the brand, the
/// CRM). One that will not parse is an error and is left as it is.
/// Returns what was written.
pub fn update(
    data_root: &Path,
    change: impl FnOnce(&mut WorkspaceConfig),
) -> Result<WorkspaceConfig> {
    migrate_legacy_layout(data_root);
    let dir = data_root.join(DOT_DIR);
    crate::secrets::with_lock(&dir, "config.json", || {
        let mut cfg = read(&dir.join("config.json"))?.unwrap_or_default();
        change(&mut cfg);
        if cfg.workspace_id.is_none() {
            cfg.workspace_id = Some(mint_workspace_id(data_root));
        }
        cfg.save(data_root)?;
        Ok(cfg)
    })
}

/// The config at `path`, None when there is none. One that will not
/// parse is an error, never "none": that would let the next write wipe it.
pub(super) fn read(path: &Path) -> Result<Option<WorkspaceConfig>> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| Error::Config(format!("{}: {}", path.display(), e)))
}

/// UUID-shaped id from sha256(data_root, time, pid) - collision-safe
/// across machines without a rand dependency.
fn mint_workspace_id(data_root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let h = format!(
        "{:x}",
        Sha256::digest(format!(
            "{}|{}|{}",
            data_root.display(),
            now,
            std::process::id()
        ))
    );
    // Hex output is pure ASCII; .get() keeps the no-string-indexing rule.
    let seg = |a: usize, b: usize| h.get(a..b).unwrap_or_default();
    format!(
        "{}-{}-{}-{}-{}",
        seg(0, 8),
        seg(8, 12),
        seg(12, 16),
        seg(16, 20),
        seg(20, 32)
    )
}

#[cfg(test)]
mod tests {
    use super::super::Product;
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mb-cfg-{}-{}", name, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    fn product(name: &str) -> Product {
        Product {
            name: name.into(),
            ad_account: "123456".into(),
        }
    }

    #[test]
    fn update_keeps_a_change_made_on_disk_since_the_caller_loaded() {
        let root = tmp("concurrent");
        let first = update(&root, |w| w.products = vec![product("A")]).unwrap();
        let id = first
            .workspace_id
            .clone()
            .expect("a new config is minted an id");
        // Another process sets the brand; this one still holds `first`.
        update(&root, |w| w.brand = Some("Fincity".into())).unwrap();
        let saved = update(&root, |w| w.products.push(product("B"))).unwrap();
        assert_eq!(saved.brand.as_deref(), Some("Fincity"));
        assert_eq!(saved.product_names(), ["A", "B"]);
        assert_eq!(saved.workspace_id.as_deref(), Some(id.as_str()));
        let on_disk = WorkspaceConfig::load(&root).unwrap();
        assert_eq!(on_disk.brand.as_deref(), Some("Fincity"));
        assert_eq!(on_disk.product_names(), ["A", "B"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn writers_at_once_lose_nothing() {
        let root = tmp("threads");
        let names: Vec<String> = (0..8).map(|i| format!("P{}", i)).collect();
        std::thread::scope(|s| {
            for n in &names {
                let root = &root;
                s.spawn(move || update(root, |w| w.products.push(product(n))).unwrap());
            }
        });
        let w = WorkspaceConfig::load(&root).unwrap();
        let mut got = w.product_names();
        got.sort();
        assert_eq!(got, names.iter().map(String::as_str).collect::<Vec<_>>());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_config_that_will_not_parse_is_never_written_over() {
        let root = tmp("garbled");
        std::fs::create_dir_all(root.join(DOT_DIR)).unwrap();
        let path = root.join(DOT_DIR).join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(update(&root, |w| w.products = vec![product("A")]).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        std::fs::remove_dir_all(&root).ok();
    }
}
