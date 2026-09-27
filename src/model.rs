//! The core: one query in, one packet out.
//!
//! Everything an agent reads comes out of here, whichever source produced it.
//! Kept deliberately small: the shared representations live here and nowhere
//! else, and every source is an operation over them.

use crate::outline::StructureItem;
use serde::Serialize;

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

/// A query is the order form. Verbs are constructors over this, not parallel
/// implementations.
#[derive(Debug, Clone)]
pub enum Query {
    /// Exact lexical search, optionally a regex.
    Lexical { text: String, regex: bool },
    /// Ranked file discovery by path and content terms.
    Path { terms: Vec<String> },
    /// Structure of one known file.
    Outline { file: String },
    /// Structured investigation: a subject, a relation, and supporting terms.
    Structural { subject: String },
}

impl Query {
    /// The human-facing query string, used in output headers and JSON.
    pub fn label(&self) -> String {
        match self {
            Query::Lexical { text, .. } => text.clone(),
            Query::Path { terms } => terms.join(" "),
            Query::Outline { file } => file.clone(),
            Query::Structural { subject } => subject.clone(),
        }
    }
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
        }
    }
}
