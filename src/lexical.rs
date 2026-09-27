//! Lexical matching: the always-available source.
//!
//! Absorbed from agentgrep's `search.rs`, reshaped onto the packet core and
//! bounded. Two properties are deliberate:
//!
//! * **Per file, then dropped.** A file is read, matched, and released before
//!   the next one, so peak memory tracks the largest file, not the corpus.
//! * **Bounded storage.** Matches are stored until the budget is spent, then
//!   counted but not kept. The reported count stays exact; the memory does not
//!   grow with the result size.
//!
//! The `rg` subprocess fast path was not carried over. It was 563 lines of
//! parsing and parity fixups for a second searcher that had to agree with this
//! one. One searcher is enough until a benchmark says otherwise.

use crate::cli::GrepArgs;
use crate::model::{Budget, Group, Hit, LineMatch, Packet};
use crate::outline::{extract_file_structure, infer_role};
use crate::scan::{FileEntry, ScanConfig, SearchScope, read_text_file};
use ignore::WalkState;
use regex::Regex;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// At or above this many matches in one file, per-symbol grouping is skipped:
/// a file this dense is better described as one file-scope group than as
/// hundreds of symbol groups.
const DENSE_SKIP_STRUCTURE: usize = 24;
/// At or above this many matches, groups themselves are capped.
const DENSE_LIMITED_GROUPING: usize = 12;
const DENSE_GROUPS_LIMIT: usize = 8;
const DENSE_OTHER_SYMBOLS_LIMIT: usize = 2;
const OTHER_SYMBOLS_LIMIT: usize = 4;

/// What the workers accumulate. One lock per matching file, which is cheap
/// beside reading and scanning it.
#[derive(Default, Debug)]
struct Accum {
    hits: Vec<Hit>,
    total_matches: usize,
    omitted_matches: usize,
    truncated: bool,
}

pub fn run_grep(root: &Path, args: &GrepArgs, budget: Budget) -> Result<Packet, String> {
    let matcher = Matcher::new(&args.query, args.regex)?;
    let scope = SearchScope {
        root,
        file_type: args.scope.file_type.as_deref(),
        glob: args.scope.glob.as_deref(),
        hidden: args.scope.hidden,
        no_ignore: args.scope.no_ignore,
        follow: !args.scope.no_follow,
    };
    let config = ScanConfig::new(&scope);

    // Streamed, not collected. The walker hands entries to the workers as it
    // finds them, so there is no file list held in memory, and the workers give
    // us the parallelism in the same stroke. The previous design materialized
    // every entry first, which cost 268 bytes per file and was the only term in
    // the tool that grew with repository size.
    let accum = Arc::new(Mutex::new(Accum::default()));
    // Global, so the budget is exact rather than per-worker.
    let spent = Arc::new(AtomicUsize::new(0));

    config.walker().build_parallel().run(|| {
        let matcher = matcher.clone();
        let args = args.clone();
        let config = config.clone();
        let accum = Arc::clone(&accum);
        let spent = Arc::clone(&spent);

        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            if !config.accepts(entry.path(), entry.path_is_symlink()) {
                return WalkState::Continue;
            }
            let file = config.entry(entry.path());
            let Some(text) = read_text_file(&file.path) else {
                return WalkState::Continue;
            };

            let already = spent.load(Ordering::Relaxed);
            let Some((stored, total_in_file, hit)) =
                scan_file(&file, &text, &matcher, &args, budget, already)
            else {
                return WalkState::Continue;
            };
            if stored > 0 {
                spent.fetch_add(stored, Ordering::Relaxed);
            }

            let mut accum = accum.lock().expect("accumulator lock");
            accum.hits.push(hit);
            accum.total_matches += total_in_file;
            if total_in_file > stored {
                accum.omitted_matches += total_in_file - stored;
                accum.truncated = true;
            }
            WalkState::Continue
        })
    });

    let Accum {
        mut hits,
        total_matches,
        omitted_matches,
        truncated,
    } = Arc::try_unwrap(accum)
        .expect("workers have finished")
        .into_inner()
        .expect("accumulator lock");

    // Rank, then order. Without this a budget would truncate an unranked list
    // and keep whichever hits happen to sort first by path.
    let ranking = crate::rank::Ranking::from_env();
    let tokens = crate::rank::query_tokens(&args.query);
    if ranking != crate::rank::Ranking::None {
        for hit in &mut hits {
            let (score, why) = crate::rank::score(hit, &args.query, &tokens, ranking);
            hit.score = score;
            hit.why = why;
        }
    }
    // Always a total order: by rank when ranking is on, by path otherwise, so
    // output never depends on which worker finished first.
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));

    let mut packet = Packet::new(args.query.clone(), args.regex, root.display().to_string());
    packet.total_files = hits.len();
    packet.total_matches = total_matches;
    packet.omitted_matches = omitted_matches;
    packet.truncated = truncated;
    packet.hits = hits;

    // Cap the list itself, because coverage is also a term that grows with the
    // repository: a generic term in a large repo matches tens of thousands of
    // files, and their names alone would blow any budget. The true total is
    // kept and reported, so nothing is lost silently.
    if let Some(max) = budget.max_hits
        && packet.hits.len() > max
    {
        packet.unlisted_files = packet.hits.len() - max;
        packet.hits.truncate(max);
        packet.truncated = true;
    }

    // Spend the detail budget last, in ranked order, once we know what the best
    // answer is. Among the files we do list, every one keeps its name. Only
    // detail is dropped, which is the cheapest thing to drop and the most
    // expensive thing to keep.
    let (summarized_hits, freed_matches) = apply_detail_budget(&mut packet.hits, budget);
    packet.summarized_hits = summarized_hits;
    packet.omitted_matches += freed_matches;
    if summarized_hits > 0 {
        packet.truncated = true;
    }
    Ok(packet)
}

/// Rough token estimate. Four characters per token is crude and consistent,
/// which is all a budget needs.
fn estimate_tokens(chars: usize) -> usize {
    chars.div_ceil(4)
}

/// What a hit's detail costs: match lines, group headers, symbol listings.
fn hit_detail_cost(hit: &Hit) -> usize {
    let matches: usize = hit.matches.iter().map(|line| line.line_text.len() + 18).sum();
    let groups = hit.groups.len() * 26;
    let symbols: usize = hit.other_symbols.iter().map(|item| item.label.len() + 26).sum();
    estimate_tokens(matches + groups + symbols)
}

/// Spend the detail budget in ranked order and return `(summarized hits, matches
/// released)`. A hit that cannot afford detail keeps its name, role, score and
/// true match count, and releases the memory its detail held.
///
/// The top hit is always shown in full even if it alone exceeds the budget, so a
/// single dense file cannot leave the caller with nothing.
fn apply_detail_budget(hits: &mut [Hit], budget: Budget) -> (usize, usize) {
    let Some(limit) = budget.max_detail_tokens else {
        return (0, 0);
    };

    let mut spent = 0usize;
    let mut summarized = 0usize;
    let mut freed = 0usize;

    for hit in hits.iter_mut() {
        let cost = hit_detail_cost(hit);
        if spent == 0 && summarized == 0 {
            spent += cost;
            continue;
        }
        if spent.saturating_add(cost) <= limit {
            spent += cost;
            continue;
        }
        freed += hit.matches.len();
        // Keep the true count on the hit before releasing the detail, so the
        // rendered summary is about this file and not about zero.
        hit.omitted_matches += hit.matches.len();
        hit.matches.clear();
        hit.groups.clear();
        hit.other_symbols.clear();
        hit.other_symbols_omitted_count = 0;
        hit.summarized = true;
        summarized += 1;
    }

    (summarized, freed)
}

/// Walk a file once for matches, then once more only if a structure sketch is
/// worth drawing. Returns `(stored, total, hit)`, or `None` when nothing
/// matched.
fn scan_file(
    entry: &FileEntry,
    text: &str,
    matcher: &Matcher,
    args: &GrepArgs,
    budget: Budget,
    spent_matches: usize,
) -> Option<(usize, usize, Hit)> {
    let path = entry.display_path();
    let mut stored: Vec<LineMatch> = Vec::new();
    let mut total = 0usize;

    for (idx, line) in text.lines().enumerate() {
        if !matcher.is_match(line) {
            continue;
        }
        total += 1;
        // `paths_only` never needs match bodies, so it stores none.
        if args.paths_only {
            continue;
        }
        if spent_matches + stored.len() < budget.max_total_matches {
            stored.push(LineMatch {
                line_number: idx + 1,
                line_text: line.to_string(),
            });
        }
    }

    if total == 0 {
        return None;
    }

    let language = infer_language(&entry.path);
    if args.paths_only {
        let hit = Hit::empty(path, infer_role(&entry.relative_path), language);
        return Some((0, total, hit));
    }

    if total >= DENSE_SKIP_STRUCTURE {
        let hit = Hit {
            groups: vec![Group {
                kind: "file-scope".to_string(),
                label: "<file scope>".to_string(),
                start_line: None,
                end_line: None,
                match_indices: (0..stored.len()).collect(),
            }],
            matches: stored.clone(),
            ..Hit::empty(path, infer_role(&entry.relative_path), language)
        };
        return Some((stored.len(), total, hit));
    }

    let structure = extract_file_structure(&entry.path, &entry.relative_path, text);
    let grouping = group_matches(&structure.items, &stored);

    let hit = Hit {
        path,
        path_bytes: entry.path_bytes_hex(),
        language: structure.language,
        role: structure.role,
        score: 0,
        why: Vec::new(),
        matches: stored.clone(),
        groups: grouping.groups,
        total_symbols: structure.items.len(),
        matched_symbol_count: grouping.matched_symbol_count,
        other_symbols: grouping.other_symbols,
        other_symbols_omitted_count: grouping.other_symbols_omitted_count,
        omitted_matches: 0,
        summarized: false,
    };
    Some((stored.len(), total, hit))
}

fn infer_language(path: &Path) -> String {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "md" => "markdown",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        _ => "text",
    }
    .to_string()
}

#[derive(Debug, Clone)]
enum Matcher {
    Literal(String),
    Pattern(Regex),
}

impl Matcher {
    fn new(query: &str, regex: bool) -> Result<Self, String> {
        if regex {
            let pattern = Regex::new(query).map_err(|err| format!("invalid regex: {err}"))?;
            Ok(Matcher::Pattern(pattern))
        } else {
            Ok(Matcher::Literal(query.to_string()))
        }
    }

    fn is_match(&self, line: &str) -> bool {
        match self {
            Matcher::Literal(needle) => line.contains(needle.as_str()),
            Matcher::Pattern(pattern) => pattern.is_match(line),
        }
    }
}

struct Grouping {
    groups: Vec<Group>,
    matched_symbol_count: usize,
    other_symbols: Vec<crate::outline::StructureItem>,
    other_symbols_omitted_count: usize,
}

#[derive(Clone, Copy)]
struct GroupingOptions {
    max_groups: usize,
    other_symbols_limit: usize,
}

fn group_matches(items: &[crate::outline::StructureItem], matches: &[LineMatch]) -> Grouping {
    let options = if matches.len() >= DENSE_LIMITED_GROUPING {
        GroupingOptions {
            max_groups: DENSE_GROUPS_LIMIT,
            other_symbols_limit: DENSE_OTHER_SYMBOLS_LIMIT,
        }
    } else {
        GroupingOptions {
            max_groups: usize::MAX,
            other_symbols_limit: OTHER_SYMBOLS_LIMIT,
        }
    };
    group_with_options(items, matches, options)
}

fn group_with_options(
    items: &[crate::outline::StructureItem],
    matches: &[LineMatch],
    options: GroupingOptions,
) -> Grouping {
    let mut symbol_groups: Vec<Group> = Vec::new();
    let mut matched_indices: Vec<usize> = Vec::new();
    let mut file_scope_matches: Vec<usize> = Vec::new();
    let mut item_idx = 0usize;
    let mut last_grouped_item_idx: Option<usize> = None;

    for (match_idx, line_match) in matches.iter().enumerate() {
        while item_idx < items.len() && items[item_idx].end_line < line_match.line_number {
            item_idx += 1;
        }

        if let Some(item) = items.get(item_idx)
            && item.start_line <= line_match.line_number
            && line_match.line_number <= item.end_line
        {
            if matched_indices.last().copied() != Some(item_idx) {
                matched_indices.push(item_idx);
                if symbol_groups.len() < options.max_groups {
                    symbol_groups.push(Group {
                        kind: item.kind.clone(),
                        label: item.label.clone(),
                        start_line: Some(item.start_line),
                        end_line: Some(item.end_line),
                        match_indices: vec![match_idx],
                    });
                    last_grouped_item_idx = Some(item_idx);
                } else {
                    file_scope_matches.push(match_idx);
                    last_grouped_item_idx = None;
                }
            } else if last_grouped_item_idx == Some(item_idx) {
                let group = symbol_groups
                    .last_mut()
                    .expect("a group exists for a grouped symbol");
                group.match_indices.push(match_idx);
            } else {
                file_scope_matches.push(match_idx);
            }
        } else {
            file_scope_matches.push(match_idx);
        }
    }

    let mut groups = Vec::with_capacity(symbol_groups.len() + 1);
    if !file_scope_matches.is_empty() {
        groups.push(Group {
            kind: "file-scope".to_string(),
            label: "<file scope>".to_string(),
            start_line: None,
            end_line: None,
            match_indices: file_scope_matches,
        });
    }
    groups.extend(symbol_groups);

    let matched_symbol_count = matched_indices.len();
    let mut other_symbols = Vec::new();
    let mut other_symbols_omitted_count = 0;
    let mut matched_iter = matched_indices.into_iter().peekable();
    for (idx, item) in items.iter().enumerate() {
        if matched_iter.peek().copied() == Some(idx) {
            matched_iter.next();
            continue;
        }
        if other_symbols.len() < options.other_symbols_limit {
            other_symbols.push(item.clone());
        } else {
            other_symbols_omitted_count += 1;
        }
    }

    Grouping {
        groups,
        matched_symbol_count,
        other_symbols,
        other_symbols_omitted_count,
    }
}
