//! moneyball-core - headless logic for the moneyball CLI.
//!
//! This crate owns snapshot loading, scoreboard math, funnel aggregation,
//! diagnostic synthesis, the advisor agent loop, and the LLM streaming
//! loop. It has no TUI dependency - the TUI and any other front-end
//! consume it as a library.

pub mod brief;
pub mod config;
pub mod crm;
pub mod error;
pub mod fetch;
pub mod llm;
pub mod logo;
pub mod meta;
pub mod provider;
pub mod secrets;
pub mod session;
pub mod snapshot;
pub mod tools;

pub use logo::LOGO;
pub use meta::{list_ad_accounts, validate_token, AdAccount};
pub use session::{Session, SessionCell, SessionMeta};

pub use config::{AppConfig, WorkspaceConfig};
pub use error::{Error, Result};
pub use llm::Client as LlmClient;
pub use provider::{ModelProviderInfo, WireApi};
pub use snapshot::Snapshot;
pub use tools::{Completion, Tool, ToolCall, ToolResult};
