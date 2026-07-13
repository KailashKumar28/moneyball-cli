//! OS-native secret storage via the `keyring` crate.
//!
//! Tokens NEVER live on disk in plaintext. macOS Keychain, Linux Secret
//! Service, Windows Credential Manager - all via the same API.

use crate::error::{Error, Result};

const SERVICE: &str = "moneyball-cli";
const META_KEY: &str = "meta-access-token";

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