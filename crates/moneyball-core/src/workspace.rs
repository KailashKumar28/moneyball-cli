//! The workspace a GUI opens: one explicit directory, and the memory of
//! the last one opened. An app launched from Finder has cwd `/` and no
//! shell environment, so none of config.rs's guesses (the cwd, the
//! walk-up, the well-known home paths, `<cwd>/moneyball-data`) apply to
//! it. The memory is the `last_workspace` key of
//! `~/.moneyball/config.json` - its own key, not the `data_root` the CLI
//! and TUI read, so remembering never changes where a shell lands.

use std::path::{Path, PathBuf};

use crate::config::{home_dir, AppConfig, WorkspaceConfig, DOT_DIR};
use crate::error::{Error, Result};

const GLOBAL_CONFIG: &str = "config.json"; // in ~/.moneyball/
const KEY: &str = "last_workspace";

/// Open the workspace at `dir`, and only that: no flag, no environment,
/// no cwd, no guessing. Err when `dir` holds no workspace config, so the
/// caller can say so instead of showing an empty one. `date` pins a past
/// snapshot (what `--date` does in a shell); None reads the latest.
pub fn open(dir: &Path, date: Option<&str>) -> Result<AppConfig> {
    let root = canonical(dir);
    let workspace = WorkspaceConfig::load(&root)?;
    Ok(AppConfig {
        data_root: root,
        date: date.map(String::from),
        workspace: Some(workspace),
        agent: false,
    })
}

/// Remember `dir` as the last opened workspace, keeping every other key
/// of `~/.moneyball/config.json`.
pub fn remember(dir: &Path) -> Result<()> {
    let home = home_dir()
        .ok_or_else(|| Error::Config("no home directory to remember the workspace in".into()))?;
    write_last(&home.join(DOT_DIR), dir)
}

/// The last workspace remembered, if any. It may have moved since:
/// `open` it to find out.
pub fn last() -> Option<PathBuf> {
    read_last(&home_dir()?.join(DOT_DIR))
}

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn read_last(global_dir: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(global_dir.join(GLOBAL_CONFIG)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(canonical(Path::new(v.get(KEY)?.as_str()?)))
}

fn write_last(global_dir: &Path, dir: &Path) -> Result<()> {
    let path = global_dir.join(GLOBAL_CONFIG);
    // A file that will not parse is never overwritten: it may hold keys
    // this code does not know.
    let mut v = match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw)
            .map_err(|e| Error::Config(format!("{}: {}", path.display(), e)))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.into()),
    };
    let root = canonical(dir);
    let (Some(keys), Some(root)) = (v.as_object_mut(), root.to_str()) else {
        return Err(Error::Config(format!(
            "cannot remember {} in {}: the path must be UTF-8 and the file a JSON object",
            dir.display(),
            path.display()
        )));
    };
    keys.insert(KEY.into(), root.into());
    crate::secrets::create_private_dir(global_dir)?;
    crate::secrets::write_private(global_dir, GLOBAL_CONFIG, serde_json::to_string_pretty(&v)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mb-ws-{}-{}", name, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn remember_keeps_other_keys_and_last_reads_it_back() {
        let global = tmp("remember");
        assert_eq!(read_last(&global), None, "nothing remembered yet");
        std::fs::write(global.join(GLOBAL_CONFIG), r#"{"theme": "dark"}"#).unwrap();
        let ws = tmp("remember-ws");
        write_last(&global, &ws).unwrap();
        write_last(&global, &ws).unwrap();
        assert_eq!(read_last(&global), Some(canonical(&ws)));
        let raw = std::fs::read_to_string(global.join(GLOBAL_CONFIG)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["theme"], "dark", "{}", raw);
        assert!(v.get("data_root").is_none(), "the CLI's key is not ours");
        std::fs::remove_dir_all(&global).ok();
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn remember_never_overwrites_a_file_it_cannot_read() {
        let global = tmp("garbled");
        std::fs::write(global.join(GLOBAL_CONFIG), "{ not json").unwrap();
        assert!(write_last(&global, &global).is_err());
        let raw = std::fs::read_to_string(global.join(GLOBAL_CONFIG)).unwrap();
        assert_eq!(raw, "{ not json");
        std::fs::remove_dir_all(&global).ok();
    }

    #[test]
    fn open_takes_the_directory_given_or_says_there_is_no_workspace() {
        let ws = tmp("open");
        let err = open(&ws, None).unwrap_err();
        assert!(matches!(err, Error::NoWorkspaceConfig { .. }), "{}", err);
        std::fs::create_dir_all(ws.join(DOT_DIR)).unwrap();
        std::fs::write(
            ws.join(DOT_DIR).join("config.json"),
            r#"{"products": [{"name": "P", "ad_account": "act_1"}]}"#,
        )
        .unwrap();
        let cfg = open(&ws, Some("2026-11-01")).unwrap();
        assert_eq!(cfg.data_root, canonical(&ws));
        assert_eq!(cfg.date.as_deref(), Some("2026-11-01"));
        assert_eq!(cfg.sessions_root(), Some(cfg.data_root.as_path()));
        std::fs::remove_dir_all(&ws).ok();
    }
}
