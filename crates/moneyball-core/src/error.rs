//! Crate-wide error type. anyhow at the call site; thiserror at boundaries.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no workspace config at {path}\ncopy crates/moneyball-core/examples/example.fincity.json (or your own) and edit it.")]
    NoWorkspaceConfig { path: String },

    #[error("no snapshot for date {date} (looked in {snap_root})")]
    NoSnapshot { date: String, snap_root: String },

    #[error("snapshot {date}/{file}: invalid JSON: {source}")]
    InvalidJson {
        date: String,
        file: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("meta api: {0}")]
    Meta(String),

    /// Meta refused the access token (code 190): expired or revoked. Every
    /// Meta call fails the same way until a new connection is made.
    #[error("meta auth: {0}")]
    MetaAuth(String),

    /// The CRM refused its credentials; `fix` is the one command that
    /// stores fresh ones (it names the spec's real secret).
    #[error("crm auth: {message}. Store a fresh one with: {fix}")]
    CrmAuth { message: String, fix: String },

    #[error("secrets: {0}")]
    Secrets(String),

    #[error("llm: {0}")]
    Llm(String),

    #[error("llm auth: {0}")]
    LlmAuth(String),
}

impl Error {
    /// Machine-readable kind for run records and the scheduler.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::MetaAuth(_) => "meta_auth",
            Error::Meta(_) => "meta",
            Error::CrmAuth { .. } => "crm_auth",
            Error::LlmAuth(_) => "llm_auth",
            Error::Llm(_) => "llm",
            Error::Secrets(_) => "secrets",
            Error::NoWorkspaceConfig { .. } | Error::Config(_) => "config",
            Error::NoSnapshot { .. } | Error::InvalidJson { .. } => "snapshot",
            Error::Io(_) | Error::Json(_) => "io",
        }
    }

    /// The sentence without its fix and, for the kinds a run record or a
    /// screen names apart (meta, crm, config), without its "kind:" prefix.
    pub fn reason(&self) -> String {
        match self {
            Error::CrmAuth { message, .. }
            | Error::MetaAuth(message)
            | Error::Meta(message)
            | Error::Config(message) => message.clone(),
            e => e.to_string(),
        }
    }

    /// The one command that fixes it, when one exists today. A leading
    /// "/" means a command inside moneyball, anything else a shell command.
    pub fn fix(&self) -> Option<String> {
        match self {
            Error::CrmAuth { fix, .. } => Some(fix.clone()),
            Error::MetaAuth(_) => Some("/setup meta".into()),
            _ => None,
        }
    }
}

/// An error as a front-end receives it: exactly `kind`, `reason` and
/// `fix` (null when no command fixes it), so `Result<T, Error>` can be a
/// command's return type as is. Never the Debug form or a source chain.
impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut view = s.serialize_struct("Error", 3)?;
        view.serialize_field("kind", self.kind())?;
        view.serialize_field("reason", &self.reason())?;
        view.serialize_field("fix", &self.fix())?;
        view.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_kind_reason_fix_and_nothing_else() {
        let auth = serde_json::to_value(Error::CrmAuth {
            message: "the CRM said 401".into(),
            fix: "moneyball crm secret lz".into(),
        })
        .unwrap();
        assert_eq!(
            auth,
            serde_json::json!({
                "kind": "crm_auth",
                "reason": "the CRM said 401",
                "fix": "moneyball crm secret lz",
            })
        );
        let plain = serde_json::to_value(Error::Config("bad date".into())).unwrap();
        assert_eq!(
            plain,
            serde_json::json!({"kind": "config", "reason": "bad date", "fix": null})
        );
    }
}
