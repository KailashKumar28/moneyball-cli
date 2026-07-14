//! OS-native secret storage via the `keyring` crate.
//!
//! Tokens NEVER live on disk in plaintext. macOS Keychain, Linux Secret
//! Service, Windows Credential Manager - all via the same API.
//!
//! Keychain layout:
//!   service = "moneyball-cli"
//!   account = "meta-access-token"            <- the Meta Marketing API token
//!   account = "llm:<provider_id>"            <- per-provider LLM API keys

use crate::error::{Error, Result};

const SERVICE: &str = "moneyball-cli";
const META_KEY: &str = "meta-access-token";

fn llm_account(provider_id: &str) -> String {
    format!("llm:{}", provider_id)
}

pub fn store_meta_token(token: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, META_KEY)
        .map_err(|e| Error::Secrets(format!("keyring entry: {}", e)))?;
    entry.set_password(token)
        .map_err(|e| Error::Secrets(format!("keyring set: {}", e)))
}

pub fn load_meta_token() -> Option<String> {
    let entry = keyring::Entry::new(SERVICE, META_KEY).ok()?;
    entry.get_password().ok()
}

pub fn clear_meta_token() -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, META_KEY)
        .map_err(|e| Error::Secrets(format!("keyring entry: {}", e)))?;
    entry.delete_credential()
        .map_err(|e| Error::Secrets(format!("keyring delete: {}", e)))
}

/// Store the API key for `provider_id` in the OS keychain. Overwrites any
/// existing entry. Provider id is the same key used in
/// `WorkspaceConfig.model_providers`.
pub fn store_llm_key(provider_id: &str, api_key: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, &llm_account(provider_id))
        .map_err(|e| Error::Secrets(format!("keyring entry: {}", e)))?;
    entry.set_password(api_key)
        .map_err(|e| Error::Secrets(format!("keyring set: {}", e)))
}

/// Read the API key for `provider_id`. Returns `None` if no entry exists
/// or the keychain is unavailable (e.g. headless test environments).
pub fn load_llm_key(provider_id: &str) -> Option<String> {
    let entry = keyring::Entry::new(SERVICE, &llm_account(provider_id)).ok()?;
    entry.get_password().ok()
}

/// Remove the API key for `provider_id`. Idempotent.
pub fn clear_llm_key(provider_id: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, &llm_account(provider_id))
        .map_err(|e| Error::Secrets(format!("keyring entry: {}", e)))?;
    // delete_credential returns NoEntry if missing - we treat that as ok.
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Secrets(format!("keyring delete: {}", e))),
    }
}