//! Ranking: turning a pile of hits into an ordered answer.
//!
//! This is the piece the packet budget depends on. "Rank before spending" is
//! meaningless without a score, and the reason it needs one is narrow and
//! concrete: without ranking, a budget truncates an unranked list, so it keeps
//! whichever hits happen to come first in path order rather than the best ones.
//!
//! The signals here are deliberately cheap, using only what the scan already
//! produced: the path, the match count, the file role, and the symbols the
//! file declares. Nothing here needs a parse or a second pass over the corpus.
//!
//! Enabled by `GRAPHGREP_RANK`. The default is `All`, because the experiment in
//! `bench/README.md` measured it at 3.8x better on median tokens-to-answer than
//! path order, with recall unchanged. `GRAPHGREP_RANK=none` restores the old
//! behaviour and is the control for further experiments.

use crate::model::Hit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ranking {
    /// Path order, as before. The control in the experiment.
    None,
    /// Does the query appear in the path.
    Path,
    /// Does the query appear in a declared symbol's name.
    Symbol,
    /// How specific the match is: fewer matches in a file ranks higher.
    Specificity,
    /// What kind of file it is: code above docs and tests.
    Role,
    /// Everything above.
    All,
}

impl Ranking {
    /// Read the variant from `GRAPHGREP_RANK`, defaulting to `All`.
    ///
    /// An unknown value falls back to the default rather than erroring, because
    /// this is a measurement knob and a typo should not break a search.
    pub fn from_env() -> Self {
        match std::env::var("GRAPHGREP_RANK").as_deref() {
            Ok("none") => Ranking::None,
            Ok("path") => Ranking::Path,
            Ok("symbol") => Ranking::Symbol,
            Ok("spec") | Ok("specificity") => Ranking::Specificity,
            Ok("role") => Ranking::Role,
            _ => Ranking::All,
        }
    }

    fn uses_path(self) -> bool {
        matches!(self, Ranking::Path | Ranking::All)
    }

    fn uses_symbol(self) -> bool {
        matches!(self, Ranking::Symbol | Ranking::All)
    }

    fn uses_specificity(self) -> bool {
        matches!(self, Ranking::Specificity | Ranking::All)
    }

    fn uses_role(self) -> bool {
        matches!(self, Ranking::Role | Ranking::All)
    }
}

/// Weights, one home for the whole table.
mod weight {
    pub const PATH_FULL: i32 = 120;
    pub const PATH_TOKEN: i32 = 25;
    pub const SYMBOL_EXACT: i32 = 60;
    pub const SYMBOL_TOKEN: i32 = 8;
    pub const SYMBOL_TOKEN_CAP: usize = 4;
    /// Match counts at or below this are treated as maximally specific.
    pub const SPECIFIC_AT: usize = 3;
    /// Match counts at or above this are treated as noise.
    pub const NOISY_AT: usize = 100;
    pub const ROLE_TOKEN: i32 = 20;
    pub const ROLE_CODE_BOOST: i32 = 20;
    pub const ROLE_DOCS_PENALTY: i32 = -25;
    pub const ROLE_TEST_PENALTY: i32 = -15;
}

/// Split a query into comparable lowercase tokens, the same rule the scan uses
/// for path matching so the two agree.
pub fn query_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

/// Score one hit and explain why, so the packet can carry its reasons.
pub fn score(hit: &Hit, query: &str, tokens: &[String], ranking: Ranking) -> (i32, Vec<String>) {
    let mut score = 0;
    let mut why = Vec::new();
    let query_lower = query.to_ascii_lowercase();
    let path_lower = hit.path.to_ascii_lowercase();

    if ranking.uses_path() && !query_lower.is_empty() {
        if path_lower.contains(&query_lower) {
            score += weight::PATH_FULL;
            why.push("path contains the whole query".to_string());
        }
        for token in tokens {
            if path_lower.contains(token.as_str()) {
                score += weight::PATH_TOKEN;
                why.push(format!("path contains '{token}'"));
            }
        }
    }

    if ranking.uses_symbol() {
        let mut exact = false;
        let mut hits = 0usize;
        // Matched symbols live in `groups` (everything that is not file scope);
        // unmatched ones live in `other_symbols`. Both are worth reading.
        let labels = hit
            .groups
            .iter()
            .filter(|group| group.kind != "file-scope")
            .map(|group| group.label.as_str())
            .chain(hit.other_symbols.iter().map(|item| item.label.as_str()));
        for label in labels {
            let label = label.to_ascii_lowercase();
            if tokens.iter().any(|token| label.contains(token.as_str())) {
                hits += 1;
            }
            if !query_lower.is_empty() && label == query_lower {
                exact = true;
            }
        }
        if exact {
            score += weight::SYMBOL_EXACT;
            why.push("a declared symbol has exactly this name".to_string());
        }
        if hits > 0 {
            let capped = hits.min(weight::SYMBOL_TOKEN_CAP);
            score += (capped as i32) * weight::SYMBOL_TOKEN;
            why.push(format!("{hits} declared symbol(s) match"));
        }
    }

    if ranking.uses_specificity() {
        let total = hit.matches.len() + hit.omitted_matches;
        if total > 0 && total <= weight::SPECIFIC_AT {
            score += 40;
            why.push(format!("specific: only {total} match(es)"));
        } else if total >= weight::NOISY_AT {
            score -= 30;
            why.push(format!("noisy: {total} matches"));
        }
    }

    if ranking.uses_role() {
        let role = hit.role.as_str();
        if tokens.iter().any(|token| role.contains(token.as_str())) {
            score += weight::ROLE_TOKEN;
            why.push(format!("role matches the query: {role}"));
        }
        match role {
            "implementation" | "auth" | "provider" | "ui" | "handler" => {
                score += weight::ROLE_CODE_BOOST;
                why.push(format!("code role: {role}"));
            }
            "docs" => {
                score += weight::ROLE_DOCS_PENALTY;
                why.push("docs penalty".to_string());
            }
            "test" => {
                score += weight::ROLE_TEST_PENALTY;
                why.push("test penalty".to_string());
            }
            _ => {}
        }
    }

    (score, why)
}
