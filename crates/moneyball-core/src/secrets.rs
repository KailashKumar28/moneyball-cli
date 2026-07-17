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
//!     "crm_keys": { "<name>": "..." } }

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
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Secrets("HOME not set".into()))?;
    Ok(home.join(".moneyball").join("auth.json"))
}

fn load_file() -> AuthFile {
    let Ok(p) = auth_path() else {
        return AuthFile::default();
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_file(f: &AuthFile) -> Result<()> {
    let p = auth_path()?;
    std::fs::create_dir_all(p.parent().expect("auth path has parent"))?;
    let body = serde_json::to_string_pretty(f)
        .map_err(|e| Error::Secrets(format!("serialize auth: {}", e)))?;
    // Atomic replace: a crash (or a concurrent reader) mid-write must
    // never leave a half-written auth.json - load_file treats parse
    // failures as an empty file, and the next save would then silently
    // wipe every stored key. Pid-unique tmp name so two processes
    // saving at once cannot clobber each other's tmp.
    let tmp = p.with_file_name(format!("auth.json.tmp.{}", std::process::id()));
    std::fs::write(&tmp, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

pub fn store_meta_token(token: &str) -> Result<()> {
    let mut f = load_file();
    f.meta_access_token = Some(token.to_string());
    save_file(&f)
}

pub fn load_meta_token() -> Option<String> {
    load_file().meta_access_token
}

pub fn clear_meta_token() -> Result<()> {
    let mut f = load_file();
    f.meta_access_token = None;
    save_file(&f)
}

/// Store the API key for `provider_id`. Overwrites any existing entry.
/// Provider id is the same key used in `WorkspaceConfig.model_providers`.
pub fn store_llm_key(provider_id: &str, api_key: &str) -> Result<()> {
    let mut f = load_file();
    f.llm_keys
        .insert(provider_id.to_string(), api_key.to_string());
    save_file(&f)
}

/// Read the API key for `provider_id`. `None` if absent.
pub fn load_llm_key(provider_id: &str) -> Option<String> {
    load_file().llm_keys.get(provider_id).cloned()
}

/// Remove the API key for `provider_id`. Idempotent.
pub fn clear_llm_key(provider_id: &str) -> Result<()> {
    let mut f = load_file();
    f.llm_keys.remove(provider_id);
    save_file(&f)
}

/// Store a CRM secret referenced from crm.toml as `secret:<name>`.
pub fn store_crm_key(name: &str, value: &str) -> Result<()> {
    let mut f = load_file();
    f.crm_keys.insert(name.to_string(), value.to_string());
    save_file(&f)
}

/// Read a CRM secret. `None` if absent.
pub fn load_crm_key(name: &str) -> Option<String> {
    load_file().crm_keys.get(name).cloned()
}

/// Remove a CRM secret. Idempotent.
pub fn clear_crm_key(name: &str) -> Result<()> {
    let mut f = load_file();
    f.crm_keys.remove(name);
    save_file(&f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hermetic round trip through the MONEYBALL_AUTH_PATH seam - the
    /// user's real ~/.moneyball/auth.json is never touched. (Replaces
    /// the old moneyball-tui keychain test that rewrote the real file.)
    #[test]
    fn round_trip_persists_via_override_path() {
        let dir =
            std::env::temp_dir().join(format!("mb-secrets-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("auth.json");
        std::env::set_var("MONEYBALL_AUTH_PATH", &file);

        store_llm_key("test_provider", "sk-test-1234").unwrap();
        store_crm_key("test_crm", "tok-5678").unwrap();
        assert_eq!(load_llm_key("test_provider").as_deref(), Some("sk-test-1234"));
        assert_eq!(load_crm_key("test_crm").as_deref(), Some("tok-5678"));
        // No stray tmp file left behind by the atomic save.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        clear_llm_key("test_provider").unwrap();
        assert_eq!(load_llm_key("test_provider"), None);

        std::env::remove_var("MONEYBALL_AUTH_PATH");
        std::fs::remove_dir_all(&dir).ok();
    }
}
