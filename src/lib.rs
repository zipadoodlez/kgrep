//! kgrep: code search and retrieval for agents.
//!
//! One shaper, several sources, flat memory. See `AGENTS.md` for the shape and
//! the reasoning; this crate is that design, not a plan for it.

pub mod cli;
pub mod find;
pub mod lexical;
pub mod model;
pub mod outline;
pub mod packet;
pub mod peak;
pub mod rank;
pub mod scan;
pub mod trace;

/// The version reported by `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
