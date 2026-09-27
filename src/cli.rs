//! The command surface.
//!
//! Four front doors over one core. The flags here are a compatibility contract
//! for humans, scripts, and kcode: keep them exact.

use clap::{ArgAction, Parser, Subcommand};

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

    /// Follow symlinks while searching.
    ///
    /// Off by default, which matches ripgrep, matches agentgrep v0.1.6 (the
    /// version kcode runs today, which never followed), and avoids reporting
    /// the same file twice under a real path and a linked one. Turn it on to
    /// search through links, accepting that a link into a large external tree
    /// becomes searchable and that duplicates can appear.
    #[arg(long)]
    pub follow: bool,

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

/// How much of a matched region `trace` expands.
///
/// It lives in `model` because it is part of what a query asks for, and is
/// re-exported here so the CLI, and kcode, can keep naming it from one place.
pub use crate::model::FullRegionMode;

impl ScopeArgs {
    /// The tree to search: the given path, or the working directory.
    fn root_or_cwd(&self) -> std::path::PathBuf {
        self.path
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().expect("current directory"))
    }

    /// The shared narrowing, translated once so every verb agrees.
    fn to_where(&self, root: impl Into<std::path::PathBuf>) -> crate::model::Where {
        crate::model::Where {
            root: root.into(),
            glob: self.glob.clone(),
            file_type: self.file_type.clone(),
            hidden: self.hidden,
            no_ignore: self.no_ignore,
            follow: self.follow,
        }
    }
}

impl GrepArgs {
    /// Turn the flags into the one query shape the library takes.
    pub fn to_query(&self) -> crate::model::Query {
        crate::model::Query {
            where_: self.scope.to_where(self.scope.root_or_cwd()),
            paths_only: self.paths_only,
            verb: crate::model::Verb::Lexical {
                text: self.query.clone(),
                regex: self.regex,
            },
        }
    }
}

impl FindArgs {
    pub fn to_query(&self) -> crate::model::Query {
        crate::model::Query {
            where_: self.scope.to_where(self.scope.root_or_cwd()),
            paths_only: self.paths_only,
            verb: crate::model::Verb::Path {
                terms: self.query_parts.clone(),
                max_files: self.max_files,
            },
        }
    }
}

impl OutlineArgs {
    pub fn to_query(&self) -> crate::model::Query {
        crate::model::Query {
            where_: self.scope.to_where(self.scope.root_or_cwd()),
            paths_only: false,
            verb: crate::model::Verb::Outline {
                file: self.file.clone(),
                max_items: self.max_items,
            },
        }
    }
}

impl TraceArgs {
    /// The DSL is parsed here, at the edge, so the library takes a finished
    /// query rather than a list of strings it would have to interpret.
    pub fn to_query(&self) -> Result<crate::model::Query, String> {
        Ok(crate::model::Query {
            where_: self.scope.to_where(self.scope.root_or_cwd()),
            paths_only: self.paths_only,
            verb: crate::model::Verb::Structural {
                query: crate::trace::parse_query(&self.terms)?,
                max_files: self.max_files,
                max_regions: self.max_regions,
                full_region: self.full_region,
            },
        })
    }
}
