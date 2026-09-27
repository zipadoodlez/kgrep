//! The command surface.
//!
//! Four front doors over one core. The flags here are a compatibility contract
//! for humans, scripts, and kcode: keep them exact.

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "kgrep",
    version,
    about = "Code search and retrieval for agents"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Exact lexical search.
    Grep(GrepArgs),
    /// Ranked file/path discovery.
    Find(FindArgs),
    /// File structure outline for a known file.
    Outline(OutlineArgs),
    /// Structured trace mode using a small relation-aware DSL.
    #[command(name = "trace", visible_alias = "smart")]
    Trace(TraceArgs),
}

/// Flags shared by every candidate-walking verb.
#[derive(Debug, Clone, Parser)]
pub struct ScopeArgs {
    /// Restrict to a known file type.
    #[arg(long = "type")]
    pub file_type: Option<String>,

    /// Restrict candidate files by glob.
    #[arg(long)]
    pub glob: Option<String>,

    /// Include hidden files.
    #[arg(long)]
    pub hidden: bool,

    /// Ignore .gitignore and related ignore files.
    #[arg(long = "no-ignore")]
    pub no_ignore: bool,

    /// Do not follow symlinks while searching.
    #[arg(long = "no-follow")]
    pub no_follow: bool,

    /// Search this root instead of the current directory.
    #[arg(long)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Parser)]
pub struct GrepArgs {
    /// Exact query to search for.
    pub query: String,

    #[command(flatten)]
    pub scope: ScopeArgs,

    /// Treat the query as a regular expression.
    #[arg(long)]
    pub regex: bool,

    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,

    /// Print only matching file paths.
    #[arg(long)]
    pub paths_only: bool,

    /// Maximum estimated tokens of match detail, across all files. File names
    /// are not charged, so coverage survives whatever this is set to, up to the
    /// separate cap on how many files are listed at all.
    #[arg(long = "max-tokens")]
    pub max_tokens: Option<usize>,

    /// Maximum matching files listed. Beyond this the packet reports the true
    /// total and stops naming them.
    #[arg(long = "max-hits")]
    pub max_hits: Option<usize>,

    /// Do not bound the packet. For scripts that genuinely want everything.
    #[arg(long)]
    pub unbounded: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct FindArgs {
    /// File/path-oriented query terms.
    #[arg(required = true)]
    pub query_parts: Vec<String>,

    #[command(flatten)]
    pub scope: ScopeArgs,

    /// Max files to return.
    #[arg(long, default_value_t = 10)]
    pub max_files: usize,

    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,

    /// Print only matching file paths.
    #[arg(long)]
    pub paths_only: bool,

    /// Print score information in human-readable output.
    #[arg(long = "debug-score", action = ArgAction::SetTrue)]
    pub debug_score: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct OutlineArgs {
    /// File path to outline.
    pub file: String,

    #[command(flatten)]
    pub scope: ScopeArgs,

    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,

    /// Maximum structure items to print. Defaults to all detected items.
    #[arg(long)]
    pub max_items: Option<usize>,

    /// Optional harness context JSON file.
    #[arg(long = "context-json")]
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, Parser)]
pub struct TraceArgs {
    /// Structured trace DSL terms, e.g. subject:auth_status relation:rendered.
    #[arg(required = true)]
    pub terms: Vec<String>,

    #[command(flatten)]
    pub scope: ScopeArgs,

    /// Max files to return.
    #[arg(long, default_value_t = 5)]
    pub max_files: usize,

    /// Max regions to return per file.
    #[arg(long, default_value_t = 6)]
    pub max_regions: usize,

    /// Preferred region expansion mode.
    #[arg(long, value_enum, default_value_t = FullRegionMode::Auto)]
    pub full_region: FullRegionMode,

    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,

    /// Print only matching file paths.
    #[arg(long)]
    pub paths_only: bool,

    /// Print parser/planner details.
    #[arg(long = "debug-plan", action = ArgAction::SetTrue)]
    pub debug_plan: bool,

    /// Print score information in human-readable output.
    #[arg(long = "debug-score", action = ArgAction::SetTrue)]
    pub debug_score: bool,

    /// Optional harness context JSON file.
    #[arg(long = "context-json")]
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FullRegionMode {
    Auto,
    Always,
    Never,
}
