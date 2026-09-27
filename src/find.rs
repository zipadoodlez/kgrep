//! Ranked file discovery: which files are about this topic.
//!
//! The other half of agentgrep's surface, absorbed onto the shaper. It shares
//! the ranking weight table with `grep` and `outline` rather than carrying its
//! own copy, which is the specific duplication agentgrep had between `find` and
//! `trace`.
//!
//! The walk streams and is parallel, like `grep`. The extra filter here is that
//! path evidence gates a candidate before its body is read, so `find` does not
//! degenerate into `grep` with a different sort.

use crate::cli::FindArgs;
use crate::model::{Budget, Hit, Packet};
use crate::outline::extract_file_structure;
use crate::rank;
use crate::scan::{ScanConfig, SearchScope, read_text_file};
use ignore::WalkState;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// How many symbols of a discovered file to show. Beyond this the count is
/// reported instead, so a thousand-symbol file cannot dominate its own entry.
const SHOWN_SYMBOLS: usize = 8;

pub fn run_find(root: &Path, args: &FindArgs, budget: Budget) -> Result<Packet, String> {
    let query = args.query_parts.join(" ");
    let tokens = rank::query_tokens(&query);
    let query_lower = query.to_ascii_lowercase();

    let scope = SearchScope {
        root,
        file_type: args.scope.file_type.as_deref(),
        glob: args.scope.glob.as_deref(),
        hidden: args.scope.hidden,
        no_ignore: args.scope.no_ignore,
        follow: !args.scope.no_follow,
    };
    let config = ScanConfig::new(&scope);
    let hits: Arc<Mutex<Vec<Hit>>> = Arc::new(Mutex::new(Vec::new()));

    config.walker().build_parallel().run(|| {
        let config = config.clone();
        let hits = Arc::clone(&hits);
        let tokens = tokens.clone();
        let query_lower = query_lower.clone();
        let query = query.clone();

        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            if !config.accepts(entry.path(), entry.path_is_symlink()) {
                return WalkState::Continue;
            }
            let file = config.entry(entry.path());

            // The gate, before the file is read: cheap, and it is what makes
            // this discovery rather than search.
            if !has_path_evidence(&query_lower, &tokens, &file.relative_path) {
                return WalkState::Continue;
            }
            let Some(text) = read_text_file(&file.path) else {
                return WalkState::Continue;
            };

            let structure = extract_file_structure(&file.path, &file.relative_path, &text);
            let labels: Vec<String> = structure
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect();
            let (score, why) = rank::score_discovery(
                &file.relative_path,
                &structure.role,
                &labels,
                &text,
                &query,
                &tokens,
            );
            if score <= 0 {
                return WalkState::Continue;
            }

            let shown = structure.items.iter().take(SHOWN_SYMBOLS).cloned().collect();
            let hit = Hit {
                path: file.display_path(),
                path_bytes: file.path_bytes_hex(),
                language: structure.language,
                role: structure.role,
                score,
                why,
                matches: Vec::new(),
                groups: Vec::new(),
                total_symbols: structure.items.len(),
                matched_symbol_count: 0,
                // For discovery, this is the symbol listing shown to the
                // caller rather than "symbols we did not group".
                other_symbols: shown,
                other_symbols_omitted_count: structure.items.len().saturating_sub(SHOWN_SYMBOLS),
                omitted_matches: 0,
                summarized: false,
            };
            hits.lock().expect("find accumulator").push(hit);
            WalkState::Continue
        })
    });

    let mut hits = Arc::try_unwrap(hits)
        .expect("workers have finished")
        .into_inner()
        .expect("find accumulator");

    // Score first, path as the tiebreak, so the order never depends on which
    // worker finished first.
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));

    let mut packet = Packet::new(query, false, root.display().to_string());
    packet.total_files = hits.len();
    packet.total_matches = 0;

    // `max_files` is this verb's own coverage cap, and it predates the shared
    // budget. Whichever is smaller wins.
    let cap = match budget.max_hits {
        Some(shared) => args.max_files.min(shared),
        None => args.max_files,
    };
    if hits.len() > cap {
        packet.unlisted_files = hits.len() - cap;
        packet.truncated = true;
        hits.truncate(cap);
    }
    packet.hits = hits;
    Ok(packet)
}

/// Whether the path says anything about the query. The whole query, or any one
/// of its tokens.
fn has_path_evidence(query_lower: &str, tokens: &[String], relative_path: &str) -> bool {
    let path_lower = relative_path.to_ascii_lowercase();
    path_lower.contains(query_lower)
        || tokens.iter().any(|token| path_lower.contains(token.as_str()))
}
