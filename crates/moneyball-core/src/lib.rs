//! moneyball-core - headless logic for the moneyball CLI.
//!
//! This crate owns snapshot loading, scoreboard math, funnel aggregation,
//! diagnostic synthesis, the advisor agent loop, and the LLM streaming
//! loop. It has no TUI dependency - the TUI and any other front-end
//! consume it as a library.

pub mod brief;
pub mod config;
pub mod error;
pub mod snapshot;

pub use config::{AppConfig, WorkspaceConfig};
pub use error::{Error, Result};
pub use snapshot::Snapshot;