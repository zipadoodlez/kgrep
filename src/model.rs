//! The core: one query in, one packet out.
//!
//! Everything an agent reads comes out of here, whichever source produced it.
//! Kept deliberately small: the shared representations live here and nowhere
//! else, and every source is an operation over them.

use crate::outline::StructureItem;
use serde::Serialize;
use std::path::PathBuf;

/// A budget bounds a result so that no tool allocates in proportion to the
/// repository. Exceeding it truncates and records what was dropped, rather
/// than growing.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// Total match records kept across all hits. Further matches are counted
    /// but not stored, so the count stays honest and memory stays flat.
    pub max_total_matches: usize,
    /// Hits kept. `None` means unbounded. The default is bounded because the
    /// file list is itself a term that grows with the repository: a generic
    /// term in a large repo matches tens of thousands of files, and their names
    /// alone would exceed any budget. The true total is always reported.
    pub max_hits: Option<usize>,
    /// Estimated tokens of *detail* the packet may spend, across all hits.
    /// Detail means match lines and symbol listings. Names of matching files
    /// are not charged here, because coverage is the information we refuse to
    /// drop quietly.
    ///
    /// `None` means unbounded, which is the deliberate opt-out. It is the
    /// reason a generic query can still ask for everything instead of being
    /// silently clipped.
    pub max_detail_tokens: Option<usize>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_total_matches: 20_000,
            max_hits: Some(500),
            // Generous against real queries (the measured median answer is 101
            // tokens) and still bounds the pathological case by roughly 19x,
            // where a generic term produces 154k tokens of output.
            max_detail_tokens: Some(8_000),
        }
    }
}

/// How much of a matched region `trace` expands into the packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum FullRegionMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// Where to look.
///
/// A filter set, not a choice: the pieces compose. A caller can ask for the
/// whole tree, or narrow it by a glob, a type or a role, and does not have to
/// pick exactly one. Splitting this into a "one of" would drop the ability to
/// combine a glob with a type, which the CLI already allows.
///
/// `root` is the resolved place to search, not a place plus a narrowing: a
/// single file is a root that happens to be a file, which is how the CLI has
/// always behaved. There is deliberately no separate `path` field, because two
/// ways to say where look is one way too many.
///
/// Only what the walker itself narrows lives here. A filter one verb applies to
/// its own candidates, such as trace's `role` or `path_hint`, stays with that
/// verb, because sharing it would mean moving it between types for no gain.
#[derive(Debug, Clone)]
pub struct Where {
    /// The tree or file to search. Everything else narrows it.
    pub root: PathBuf,
    /// A file glob such as `**/*.rs`.
    pub glob: Option<String>,
    /// A language or ripgrep type such as `rs`.
    pub file_type: Option<String>,
    pub hidden: bool,
    pub no_ignore: bool,
    pub follow: bool,
}

impl Where {
    /// The whole of `root`, narrowed by nothing.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            glob: None,
            file_type: None,
            hidden: false,
            no_ignore: false,
            follow: false,
        }
    }
}

/// What is being asked. Exactly one of these is a query's shape, which is why
/// it is a choice rather than a bag of every verb's settings at once.
///
/// Each variant carries that verb's own settings, including its caps, because
/// they are the verb's and not the walker's. `Lexical` has none: its cap is the
/// shared `Budget`.
#[derive(Debug, Clone)]
pub enum Verb {
    /// Exact lexical search, optionally a regex.
    Lexical { text: String, regex: bool },
    /// Ranked file discovery by path and content terms.
    Path {
        terms: Vec<String>,
        max_files: usize,
    },
    /// Structure of one known file, resolved within `where_.root`.
    ///
    /// The file stays here rather than moving into `Where`, because an outline
    /// resolves a name relative to the search root, so the two are genuinely
    /// different things rather than two spellings of one.
    Outline {
        file: String,
        /// Cap on listed items. `None` means all of them.
        max_items: Option<usize>,
    },
    /// Structured investigation: a subject, a relation, and supporting terms.
    Structural {
        query: StructuralQuery,
        /// Cap on files listed.
        max_files: usize,
        /// Cap on regions reported per file.
        max_regions: usize,
        /// How far to expand each matched region.
        full_region: FullRegionMode,
    },
}

/// A query is the order form: where to look, what to ask, and whether names are
/// enough.
///
/// Verbs are constructors over this, not parallel implementations. The shared
/// part is `where_` and `paths_only`; the per-verb settings live inside `Verb`,
/// so this cannot drift into a bag holding four verbs' fields at once.
#[derive(Debug, Clone)]
pub struct Query {
    /// Trailing underscore because `where` is a keyword.
    pub where_: Where,
    /// Names only, no match bodies.
    ///
    /// This is part of the question rather than the printing, so it lives here.
    /// The packet records it too, so a renderer can honour it without being
    /// told twice.
    pub paths_only: bool,
    pub verb: Verb,
}

impl Query {
    /// The root to search, which is the only thing every verb needs.
    pub fn root(&self) -> &std::path::Path {
        &self.where_.root
    }

    /// The human-facing query string, used in output headers and JSON.
    pub fn label(&self) -> String {
        match &self.verb {
            Verb::Lexical { text, .. } => text.clone(),
            Verb::Path { terms, .. } => terms.join(" "),
            // An outline's target, which is what a reader would call the query.
            Verb::Outline { file, .. } => file.clone(),
            Verb::Structural { query, .. } => query.label(),
        }
    }
}

/// Printing choices.
///
/// These change how an answer is presented, never what was collected, which is
/// why they are separate from `Query`. Keeping them out of the CLI argument
/// structs is what lets rendering be independent of the command line.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Print the ranking score and the reasons behind it.
    pub debug_score: bool,
}

/// What a `trace` asks for, once the DSL has been parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralQuery {
    pub subject: String,
    pub relation: Relation,
    pub support: Vec<String>,
    /// Restrict by file role, for example `code` or `test`.
    pub kind: Option<String>,
    /// Restrict to a subtree by substring.
    pub path_hint: Option<String>,
}

/// The relation between the subject and what the caller is looking for. This is
/// the whole reason `trace` exists: `grep` can find a name, only a relation can
/// say what about the name matters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relation {
    Defined,
    CalledFrom,
    TriggeredFrom,
    Rendered,
    Populated,
    ComesFrom,
    Handled,
    Implementation,
    Custom(String),
}

impl StructuralQuery {
    /// The human-facing label, used in output headers and JSON.
    pub fn label(&self) -> String {
        format!(
            "subject:{} relation:{}",
            self.subject,
            self.relation.as_str()
        )
    }
}

impl Relation {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "defined" | "definition" => Self::Defined,
            "called_from" | "calledfrom" | "callers" => Self::CalledFrom,
            "triggered_from" | "triggeredfrom" | "trigger" => Self::TriggeredFrom,
            "rendered" | "render" | "drawn" => Self::Rendered,
            "populated" | "populate" | "set" | "assigned" => Self::Populated,
            "comes_from" | "comesfrom" | "source" | "origin" => Self::ComesFrom,
            "handled" | "handler" | "handles" => Self::Handled,
            "implementation" | "implemented" => Self::Implementation,
            other => Self::Custom(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Defined => "defined",
            Self::CalledFrom => "called_from",
            Self::TriggeredFrom => "triggered_from",
            Self::Rendered => "rendered",
            Self::Populated => "populated",
            Self::ComesFrom => "comes_from",
            Self::Handled => "handled",
            Self::Implementation => "implementation",
            Self::Custom(value) => value.as_str(),
        }
    }
}

/// A scored span inside a file, reported by `trace`.
///
/// A region is where a subject mention lives and what surrounds it, which is
/// what makes `trace` worth having over `grep`: the answer is a place to read,
/// not a list of lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub kind: String,
    pub label: String,
    pub start_line: usize,
    pub end_line: usize,
    pub score: i32,
    pub why: Vec<String>,
    /// The lines shown for this region, already trimmed.
    pub lines: Vec<LineMatch>,
}

/// One matching line inside a hit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LineMatch {
    pub line_number: usize,
    pub line_text: String,
}

/// A group of matches sharing an enclosing symbol, or the file scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub kind: String,
    pub label: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    /// Indices into the hit's `matches`.
    pub match_indices: Vec<usize>,
}

/// One thing we found: a file, its structure, its matches, and why it ranked
/// where it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub path_bytes: Option<String>,
    pub language: String,
    pub role: String,
    pub score: i32,
    pub why: Vec<String>,
    pub matches: Vec<LineMatch>,
    pub groups: Vec<Group>,
    pub total_symbols: usize,
    pub matched_symbol_count: usize,
    pub other_symbols: Vec<StructureItem>,
    pub other_symbols_omitted_count: usize,
    /// Matches that existed but were not stored because the budget was spent.
    pub omitted_matches: usize,
    /// This hit's detail was dropped to stay inside the packet budget. The
    /// path, role, score and true match count survive; the match lines and
    /// symbol listing do not, and the memory they held has been released.
    pub summarized: bool,
    /// Scored spans, reported by `trace` and empty for the other verbs.
    pub regions: Vec<Region>,
}

impl Hit {
    pub fn empty(path: String, role: String, language: String) -> Self {
        Self {
            path,
            path_bytes: None,
            language,
            role,
            score: 0,
            why: Vec::new(),
            matches: Vec::new(),
            groups: Vec::new(),
            total_symbols: 0,
            matched_symbol_count: 0,
            other_symbols: Vec::new(),
            other_symbols_omitted_count: 0,
            omitted_matches: 0,
            summarized: false,
            regions: Vec::new(),
        }
    }
}

/// The finished answer.
#[derive(Debug, Clone)]
pub struct Packet {
    pub query: String,
    pub regex: bool,
    pub root: String,
    pub hits: Vec<Hit>,
    pub total_files: usize,
    pub total_matches: usize,
    /// Matches counted but not stored, across all hits.
    pub omitted_matches: usize,
    /// Hits whose detail was dropped for the budget. Their paths are still
    /// reported, so coverage survives even when detail does not.
    pub summarized_hits: usize,
    /// Matching files beyond `max_hits`, not listed at all. They are counted in
    /// `total_files`, which always reports the truth.
    pub unlisted_files: usize,
    pub truncated: bool,
    /// The query asked for names only, so no match bodies were stored. Recorded
    /// here because the renderer would otherwise have to be told a second time,
    /// and the two copies could disagree.
    pub paths_only: bool,
}

impl Packet {
    pub fn new(query: String, regex: bool, root: String) -> Self {
        Self {
            query,
            regex,
            root,
            hits: Vec::new(),
            total_files: 0,
            total_matches: 0,
            omitted_matches: 0,
            summarized_hits: 0,
            unlisted_files: 0,
            truncated: false,
            paths_only: false,
        }
    }
}
