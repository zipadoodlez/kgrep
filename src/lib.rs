//! graphgrep: code search and retrieval for agents.
//!
//! One pipeline, one core, four front doors. See `AGENTS.md` for the shape and
//! the reasoning; this crate is that design, not a plan for it.

pub mod cli;

/// The version reported by `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
