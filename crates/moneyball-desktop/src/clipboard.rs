//! Copy text for the "In Terminal" button. Done in Rust so the webview
//! needs no clipboard permission.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::AppError;

/// macOS's own clipboard tool, by absolute path: an app launched from
/// Finder has a bare PATH, and nothing else should answer to this name.
const PBCOPY: &str = "/usr/bin/pbcopy";

fn copy(text: &str) -> std::io::Result<()> {
    let mut child = Command::new(PBCOPY).stdin(Stdio::piped()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    child.wait()?;
    Ok(())
}

#[tauri::command(async)]
pub fn copy_text(text: String) -> Result<(), AppError> {
    copy(&text).map_err(|e| AppError::new("io", format!("Could not copy: {}", e)))
}
