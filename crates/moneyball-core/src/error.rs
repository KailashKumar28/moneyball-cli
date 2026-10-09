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
    #[error("crm auth: {message}")]
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

    /// The one command that fixes it, when one exists today.
    pub fn fix(&self) -> Option<String> {
        match self {
            Error::CrmAuth { fix, .. } => Some(fix.clone()),
            _ => None,
        }
    }
}
