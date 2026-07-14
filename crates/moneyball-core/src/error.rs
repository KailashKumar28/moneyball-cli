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

    #[error("secrets: {0}")]
    Secrets(String),

    #[error("llm: {0}")]
    Llm(String),

    #[error("llm auth: {0}")]
    LlmAuth(String),
}