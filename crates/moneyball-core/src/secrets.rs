//! Secret storage - `~/.moneyball/auth.json` with 0600 permissions.
//!
//! Same pattern as codex (`~/.codex/auth.json`) and the gh CLI: a
//! user-only-readable dotfile. We deliberately moved OFF the OS keychain:
//! macOS ties keychain ACLs to the binary's code signature, and a locally
//! built (unsigned) binary gets a fresh identity on every rebuild - so
//! users were re-prompted to allow keychain access after every install.
//! A 0600 file has the same practical protection for a single-user dev
//! machine and zero prompts.
//!
//! Layout:
//!   { "meta_access_token": "...", "llm_keys": { "<provider_id>": "..." },
//!     "workspaces": { "<workspace_id>": { "crm_keys": { "<name>": "..." } } },
//!     "crm_keys": { "<name>": "..." } }
//!
//! Meta and the model are the person's; a CRM key is the client's, so it
//! lives under its workspace - two clients on the same CRM never share or
//! overwrite a key. The top-level `crm_keys` is the legacy global store,
//! still read as a fallback and never written.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meta_access_token: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    llm_keys: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    workspaces: HashMap<String, WorkspaceKeys>,
    /// Legacy, from before per-workspace keys: read, never written.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    crm_keys: HashMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkspaceKeys {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    crm_keys: HashMap<String, String>,
}

/// `~/.moneyball/auth.json` - global (not per-workspace): keys belong to
/// the user, not to a data directory.
pub fn auth_path() -> Result<PathBuf> {
    // Test seam: hermetic tests must never touch the user's real
    // credentials (ARCHITECTURE gate 3).
    if let Some(p) = std::env::var_os("MONEYBALL_AUTH_PATH") {
        return Ok(PathBuf::from(p));
    }
    let home = crate::config::home_dir().ok_or_else(|| Error::Secrets("HOME not set".into()))?;
    Ok(home.join(".moneyball").join("auth.json"))
}

/// A missing file is empty. An unreadable one is an error, never "empty":
/// treating it as empty let the next save silently wipe every stored key.
fn load_file() -> Result<AuthFile> {
    let p = auth_path()?;
    match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| {
            Error::Secrets(format!(
                "{} is unreadable ({}) - fix it or move it aside; your stored keys were not touched",
                p.display(),
                e
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AuthFile::default()),
        Err(e) => Err(e.into()),
    }
}

/// Every write goes through here: one process at a time (a lock file beside
/// auth.json, so the TUI and the CLI never lose each other's update), a
/// strict load (never save over a file we could not read), and an atomic,
/// owner-only write.
fn update(change: impl FnOnce(&mut AuthFile)) -> Result<()> {
    let p = auth_path()?;
    let dir = p.parent().expect("auth path has parent");
    std::fs::create_dir_all(dir)?;
    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("auth.json")
        .to_string();
    let lock = std::fs::File::create(dir.join(format!("{}.lock", name)))?;
    lock.lock()?;
    let mut f = load_file()?;
    change(&mut f);
    let body = serde_json::to_string_pretty(&f)
        .map_err(|e| Error::Secrets(format!("serialize auth: {}", e)))?;
    write_private(dir, &name, body)?;
    Ok(())
}

/// Atomic owner-only (0600) write for anything holding secrets or raw
/// contact PII (auth.json, leads.json, crm.json, crm_contacts.json). The
/// temp file is CREATED 0600 (a chmod after writing left a umask-readable
/// window) and named per process, so two writers never share one.
pub(crate) fn write_private(dir: &std::path::Path, name: &str, body: String) -> Result<PathBuf> {
    use std::io::Write as _;
    let path = dir.join(name);
    let tmp = dir.join(format!("{}.tmp.{}", name, std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            opts.mode(0o600);
            let file = opts.open(&tmp)?;
            // A stale tmp from a crash keeps its old mode: set it anyway.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            (&file).write_all(body.as_bytes())?;
        }
        #[cfg(not(unix))]
        opts.open(&tmp)?.write_all(body.as_bytes())?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn store_meta_token(token: &str) -> Result<()> {
    update(|f| f.meta_access_token = Some(token.to_string()))
}

pub fn load_meta_token() -> Option<String> {
    load_file().ok()?.meta_access_token
}

pub fn clear_meta_token() -> Result<()> {
    update(|f| f.meta_access_token = None)
}

/// Store the API key for `provider_id`. Overwrites any existing entry.
/// Provider id is the same key used in `WorkspaceConfig.model_providers`.
pub fn store_llm_key(provider_id: &str, api_key: &str) -> Result<()> {
    update(|f| {
        f.llm_keys
            .insert(provider_id.to_string(), api_key.to_string());
    })
}

/// Read the API key for `provider_id`. `None` if absent.
pub fn load_llm_key(provider_id: &str) -> Option<String> {
    load_file().ok()?.llm_keys.get(provider_id).cloned()
}

/// Remove the API key for `provider_id`. Idempotent.
pub fn clear_llm_key(provider_id: &str) -> Result<()> {
    update(|f| {
        f.llm_keys.remove(provider_id);
    })
}

/// Store a CRM secret (crm.toml's `secret:<name>`) for one workspace.
pub fn store_crm_key(workspace: &str, name: &str, value: &str) -> Result<()> {
    update(|f| {
        f.workspaces
            .entry(workspace.to_string())
            .or_default()
            .crm_keys
            .insert(name.to_string(), value.to_string());
    })
}

/// Read a CRM secret: the workspace's own first, then the legacy global
/// one, so a setup from before per-workspace keys keeps working.
pub fn load_crm_key(workspace: Option<&str>, name: &str) -> Option<String> {
    let f = load_file().ok()?;
    workspace
        .and_then(|w| f.workspaces.get(w)?.crm_keys.get(name).cloned())
        .or_else(|| f.crm_keys.get(name).cloned())
}

/// Remove a workspace's CRM secret. Idempotent.
pub fn clear_crm_key(workspace: &str, name: &str) -> Result<()> {
    update(|f| {
        if let Some(w) = f.workspaces.get_mut(workspace) {
            w.crm_keys.remove(name);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hermetic round trip through the MONEYBALL_AUTH_PATH seam - the
    /// user's real ~/.moneyball/auth.json is never touched. (Replaces
    /// the old moneyball-tui keychain test that rewrote the real file.)
    #[test]
    fn round_trip_persists_via_override_path() {
        let dir = std::env::temp_dir().join(format!("mb-secrets-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("auth.json");
        std::env::set_var("MONEYBALL_AUTH_PATH", &file);

        store_llm_key("test_provider", "sk-test-1234").unwrap();
        store_crm_key("ws-a", "test_crm", "tok-5678").unwrap();
        assert_eq!(
            load_llm_key("test_provider").as_deref(),
            Some("sk-test-1234")
        );
        assert_eq!(
            load_crm_key(Some("ws-a"), "test_crm").as_deref(),
            Some("tok-5678")
        );
        // Two clients on the same CRM keep their own key under one name.
        store_crm_key("ws-b", "test_crm", "tok-other").unwrap();
        assert_eq!(
            load_crm_key(Some("ws-a"), "test_crm").as_deref(),
            Some("tok-5678")
        );
        assert_eq!(
            load_crm_key(Some("ws-b"), "test_crm").as_deref(),
            Some("tok-other")
        );
        assert_eq!(load_crm_key(Some("ws-c"), "test_crm"), None);
        clear_crm_key("ws-b", "test_crm").unwrap();
        assert_eq!(load_crm_key(Some("ws-b"), "test_crm"), None);
        // No stray tmp file left behind by the atomic save (the lock file stays).
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.contains(".tmp")), "{:?}", names);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        clear_llm_key("test_provider").unwrap();
        assert_eq!(load_llm_key("test_provider"), None);

        // A file we cannot read is never saved over: the keys in it survive.
        std::fs::write(&file, "{ not json").unwrap();
        let err = store_llm_key("other", "sk-x").unwrap_err().to_string();
        assert!(err.contains("unreadable"), "{}", err);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");

        std::env::remove_var("MONEYBALL_AUTH_PATH");
        std::fs::remove_dir_all(&dir).ok();
    }
}
