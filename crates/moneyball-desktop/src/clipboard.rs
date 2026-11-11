//! Copy text for the "In Terminal" button. Done in Rust so the webview
//! needs no clipboard permission.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::AppError;

/// macOS's own clipboard tool, by absolute path: an app launched from
/// Finder has a bare PATH, and nothing else should answer to this name.
const PBCOPY: &str = "/usr/bin/pbcopy";

/// Pipe `text` into `program`; a non-zero exit is a failure, so the
/// button never says "Copied" over an empty clipboard.
fn copy(program: &str, text: &str) -> std::io::Result<()> {
    let mut child = Command::new(program).stdin(Stdio::piped()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "pbcopy ended with {}",
            status
        )));
    }
    Ok(())
}

#[tauri::command(async)]
pub fn copy_text(text: String) -> Result<(), AppError> {
    copy(PBCOPY, &text).map_err(|e| AppError::new("io", format!("Could not copy: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_that_exits_non_zero_is_a_failed_copy() {
        assert!(copy("/usr/bin/true", "").is_ok());
        assert!(copy("/usr/bin/false", "").is_err());
        assert!(copy("/nonexistent/pbcopy", "x").is_err());
    }
}
