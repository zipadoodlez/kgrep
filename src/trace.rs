//! `trace`: relation-aware investigation.
//!
//! The fourth verb and the one with no equivalent elsewhere. `grep` finds a
//! name; `trace` says what about the name matters, which is why the query has a
//! relation in it. `subject:auth_status relation:rendered` is a different
//! question from searching for `auth_status`, and the answer is a place to
//! read rather than a list of lines.
//!
//! Absorbed from agentgrep's `smart_engine.rs` and `smart_dsl.rs`, reshaped onto
//! the shaper: the walk streams and is parallel like the other verbs, and the
//! packet bound applies.
//!
//! One deliberate divergence: agentgrep's `--context-json` familiarity is not
//! applied yet. That is harness state, which is Stage 6, and half-porting it
//! would put the seam in the wrong place. The flag is accepted so the CLI
//! surface is unchanged, and this note is the record of the gap.

use crate::model::{
    Budget, FullRegionMode, Hit, LineMatch, Packet, Query, Region, Relation, StructuralQuery, Verb,
};
use crate::outline::{StructureItem, extract_file_structure};
use crate::rank;
use crate::scan::{ScanConfig, SearchScope, read_text_file};
use ignore::WalkState;
use std::sync::{Arc, Mutex};

/// A region longer than this is shown as its subject lines rather than its
/// whole body, so one large function cannot swallow the packet.
const REGION_EXPAND_LINES: usize = 40;

/// Parse the `trace` DSL: `subject:` and `relation:` are required, the rest
/// narrow.
pub fn parse_query<I, S>(terms: I) -> Result<StructuralQuery, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut subject = None;
    let mut relation = None;
    let mut support = Vec::new();
    let mut kind = None;
    let mut path_hint = None;

    for raw in terms {
        let raw = raw.as_ref().trim();
        if raw.is_empty() {
            continue;
        }
        let Some((key, value)) = raw.split_once(':').or_else(|| raw.split_once('=')) else {
            return Err(format!("invalid term: {raw} (expected key:value)"));
        };
        let key = key.trim().to_ascii_lowercase().replace('-', "_");
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("empty value for '{key}'"));
        }
        match key.as_str() {
            "subject" => {
                if subject.is_some() {
                    return Err("duplicate key: subject".to_string());
                }
                subject = Some(value.to_string());
            }
            "relation" => {
                if relation.is_some() {
                    return Err("duplicate key: relation".to_string());
                }
                relation = Some(Relation::parse(value));
            }
            "support" => support.push(value.to_string()),
            "kind" => {
                if kind.is_some() {
                    return Err("duplicate key: kind".to_string());
                }
                kind = Some(value.to_string());
            }
            "path" | "path_hint" | "pathhint" => {
                if path_hint.is_some() {
                    return Err("duplicate key: path_hint".to_string());
                }
                path_hint = Some(value.to_string());
            }
            other => return Err(format!("unknown key: {other}")),
        }
    }

    Ok(StructuralQuery {
        subject: subject.ok_or("subject:<value> is required")?,
        relation: relation.ok_or("relation:<value> is required")?,
        support,
        kind,
        path_hint,
    })
}

/// The words that signal each relation. Cheap, and deliberately broad: a
/// relation is a hint about where to look, not a proof.
fn relation_terms(relation: &Relation) -> Vec<String> {
    let terms: &[&str] = match relation {
        Relation::Defined => &["fn ", "struct ", "enum ", "trait ", "def ", "class "],
        Relation::CalledFrom => &["call", "invoke", "dispatch", "spawn", "run("],
        Relation::TriggeredFrom => &["trigger", "event", "emit", "publish", "notify"],
        Relation::Rendered => &["render", "draw", "paint", "display", "draw(", "ui"],
        Relation::Populated => &["set", "assign", "init", "populate", "update", "="],
        Relation::ComesFrom => &["import", "use ", "from ", "require", "source", "config"],
        Relation::Handled => &["handle", "match ", "if ", "else", "catch", "on_"],
        Relation::Implementation => &["impl", "implement", "fn ", "def "],
        Relation::Custom(value) => return vec![value.to_ascii_lowercase()],
    };
    terms.iter().map(|term| term.to_string()).collect()
}

pub fn run_trace(query: &Query, budget: Budget) -> Result<Packet, String> {
    let root = query.root();
    let paths_only = query.paths_only;
    let scope = SearchScope::from_where(&query.where_);
    // Everything the shared part of the query holds is read before `query` is
    // rebound to the structural part, so the rest of this function reads the
    // same way it always did.
    let Verb::Structural {
        query,
        max_files,
        max_regions,
        full_region,
    } = &query.verb
    else {
        return Err("trace needs a structural query".to_string());
    };
    let max_files = *max_files;
    let max_regions = *max_regions;
    let full_region = *full_region;
    let subject_lower = query.subject.to_ascii_lowercase();
    let subject_stripped = strip_punctuation(&subject_lower);
    let subject_tokens = rank::query_tokens(&query.subject);
    let terms = relation_terms(&query.relation);
    let support: Vec<String> = query
        .support
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let path_hint = query.path_hint.as_ref().map(|s| s.to_ascii_lowercase());
    let kind = query.kind.clone();

    let config = ScanConfig::new(&scope);
    let hits: Arc<Mutex<Vec<Hit>>> = Arc::new(Mutex::new(Vec::new()));

    config.walker().build_parallel().run(|| {
        let config = config.clone();
        let hits = Arc::clone(&hits);
        let subject_lower = subject_lower.clone();
        let subject_stripped = subject_stripped.clone();
        let subject_tokens = subject_tokens.clone();
        let terms = terms.clone();
        let support = support.clone();
        let path_hint = path_hint.clone();
        let kind = kind.clone();
        let query = query.clone();

        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            if !config.accepts(entry.path()) {
                return WalkState::Continue;
            }
            let file = config.entry(entry.path());
            let relative_lower = file.relative_path.to_ascii_lowercase();

            if let Some(hint) = &path_hint
                && !relative_lower.contains(hint)
            {
                return WalkState::Continue;
            }
            let role = crate::outline::infer_role(&file.relative_path);
            if kind_excludes(kind.as_deref(), &role) {
                return WalkState::Continue;
            }
            let Some(text) = read_text_file(&file.path) else {
                return WalkState::Continue;
            };
            let text_lower = text.to_ascii_lowercase();
            if !subject_present(
                &relative_lower,
                &text_lower,
                &subject_lower,
                &subject_stripped,
                &subject_tokens,
            ) {
                return WalkState::Continue;
            }

            let structure = extract_file_structure(&file.path, &file.relative_path, &text);
            let lines: Vec<&str> = text.lines().collect();
            let mentions: Vec<LineMatch> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.to_ascii_lowercase().contains(&subject_lower))
                .map(|(idx, line)| LineMatch {
                    line_number: idx + 1,
                    line_text: line.to_string(),
                })
                .collect();
            // No early return when there are no literal mentions: a symbol whose
            // *name* matches the subject is a region even when the body never
            // repeats the name, and that is the whole point of
            // `relation:defined`. Bailing here was the second half of the same
            // bug as the gate, and the regions check below is the real test.
            let mut regions = build_regions(
                &structure.items,
                &mentions,
                &lines,
                &relative_lower,
                &subject_lower,
                &subject_tokens,
                &terms,
                &support,
                full_region,
                &query.relation,
            );
            if regions.is_empty() {
                return WalkState::Continue;
            }
            regions.sort_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then_with(|| a.start_line.cmp(&b.start_line))
            });
            let best = regions.first().map(|region| region.score).unwrap_or(0);
            regions.truncate(max_regions);

            let mut score = 0;
            let mut why = Vec::new();
            score += (mentions.len() as i32) * 5;
            why.push(format!("subject mentions: {}", mentions.len()));
            if path_echoes_subject(&relative_lower, &subject_lower, &subject_tokens) {
                score += match query.relation {
                    Relation::Defined | Relation::Implementation => 140,
                    _ => 60,
                };
                why.push("path names the subject".to_string());
            }
            // Relation terms are looked for in the path and in declared symbol
            // names, not anywhere in the body. Scanning the whole body makes
            // the signal a proxy for file size, which is how a large test file
            // outranked the definition it was testing.
            let relation_hits = terms
                .iter()
                .filter(|term| {
                    relative_lower.contains(term.as_str())
                        || structure
                            .items
                            .iter()
                            .any(|item| item.label.to_ascii_lowercase().contains(term.as_str()))
                })
                .count();
            if relation_hits > 0 {
                score += (relation_hits as i32) * 20;
                why.push(format!("relation context: {relation_hits}"));
            }
            let support_hits = support
                .iter()
                .filter(|term| text_lower.contains(term.as_str()))
                .count();
            if support_hits > 0 {
                score += (support_hits as i32) * 10;
                why.push(format!("support terms: {support_hits}"));
            }
            match role.as_str() {
                "implementation" | "auth" | "provider" | "ui" | "handler" => {
                    score += 25;
                    why.push(format!("code role: {role}"));
                }
                "docs" => {
                    score -= 50;
                    why.push("docs penalty".to_string());
                }
                "test" => {
                    score -= 20;
                    why.push("test penalty".to_string());
                }
                _ => {}
            }
            if path_hint.is_some() {
                score += 30;
                why.push("path hint matched".to_string());
            }
            score += best / 2;
            why.push(format!("best region: {best}"));

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
                other_symbols: Vec::new(),
                other_symbols_omitted_count: 0,
                omitted_matches: 0,
                summarized: false,
                regions,
            };
            hits.lock().expect("trace accumulator").push(hit);
            WalkState::Continue
        })
    });

    let mut hits = Arc::try_unwrap(hits)
        .expect("workers have finished")
        .into_inner()
        .expect("trace accumulator");

    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));

    let mut packet = Packet::new(query.label(), false, root.display().to_string());
    packet.paths_only = paths_only;
    packet.total_files = hits.len();
    let cap = budget
        .max_hits
        .map_or(max_files, |shared| max_files.min(shared));
    if hits.len() > cap {
        packet.unlisted_files = hits.len() - cap;
        packet.truncated = true;
        hits.truncate(cap);
    }
    packet.hits = hits;
    Ok(packet)
}

fn subject_present(
    relative_lower: &str,
    text_lower: &str,
    subject_lower: &str,
    subject_stripped: &str,
    subject_tokens: &[String],
) -> bool {
    if relative_lower.contains(subject_lower) || text_lower.contains(subject_lower) {
        return true;
    }
    // Punctuation-stripped, because a subject written `mcp_call` is usually
    // declared as `McpCallInput`. Without this the file that *defines* the
    // subject is rejected before its symbols are ever considered, which is how
    // a test that merely mentions the name ended up outranking the definition.
    if !subject_stripped.is_empty()
        && (strip_punctuation(relative_lower).contains(subject_stripped)
            || strip_punctuation(text_lower).contains(subject_stripped))
    {
        return true;
    }
    let parts: Vec<String> = subject_lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect();
    !parts.is_empty()
        && parts.iter().all(|part| text_lower.contains(part.as_str()))
        && !subject_tokens.is_empty()
}

/// Should this file be skipped by `kind:`? `code` excludes documentation, and
/// any other value matches the inferred role by substring.
fn kind_excludes(kind: Option<&str>, role: &str) -> bool {
    match kind {
        None => false,
        Some("code") => role == "docs",
        Some(wanted) => !role.contains(wanted),
    }
}

fn path_echoes_subject(relative_lower: &str, subject_lower: &str, tokens: &[String]) -> bool {
    relative_lower.contains(subject_lower)
        || tokens
            .iter()
            .any(|token| relative_lower.contains(token.as_str()))
}

/// Build scored regions from the subject mentions, one per enclosing symbol.
#[allow(clippy::too_many_arguments)]
fn build_regions(
    items: &[StructureItem],
    mentions: &[LineMatch],
    lines: &[&str],
    relative_lower: &str,
    subject_lower: &str,
    subject_tokens: &[String],
    relation_terms: &[String],
    support: &[String],
    full_region: FullRegionMode,
    relation: &Relation,
) -> Vec<Region> {
    let mut regions = Vec::new();
    let mut index = 0usize;

    while index < mentions.len() {
        let mention = &mentions[index];
        let enclosing = items.iter().find(|item| {
            item.start_line <= mention.line_number && mention.line_number <= item.end_line
        });

        let (kind, label, start, end) = match enclosing {
            Some(item) => (
                item.kind.clone(),
                item.label.clone(),
                item.start_line,
                item.end_line,
            ),
            None => (
                "file-scope".to_string(),
                "<file scope>".to_string(),
                mention.line_number,
                mention.line_number,
            ),
        };

        // Collect every mention inside this region.
        let mut inside = Vec::new();
        while index < mentions.len() {
            let candidate = &mentions[index];
            if candidate.line_number >= start && candidate.line_number <= end {
                inside.push(candidate.clone());
                index += 1;
            } else {
                break;
            }
        }
        if inside.is_empty() {
            continue;
        }

        let span = end.saturating_sub(start) + 1;
        let expand = match full_region {
            FullRegionMode::Always => true,
            FullRegionMode::Never => false,
            FullRegionMode::Auto => span <= REGION_EXPAND_LINES,
        };
        let shown = if expand {
            let body: Vec<LineMatch> = lines
                .iter()
                .enumerate()
                .filter(|(idx, _)| {
                    let line = idx + 1;
                    line >= start && line <= end
                })
                .take(REGION_EXPAND_LINES)
                .map(|(idx, line)| LineMatch {
                    line_number: idx + 1,
                    line_text: line.to_string(),
                })
                .collect();
            if body.len() > inside.len() {
                body
            } else {
                inside.clone()
            }
        } else {
            inside.clone()
        };

        let mut score = 20;
        let mut why = Vec::new();
        score += (inside.len() as i32) * 8;
        why.push(format!("subject mentions: {}", inside.len()));
        let label_lower = label.to_ascii_lowercase();
        if label_matches_subject(&label_lower, subject_lower, subject_tokens) {
            score += 30;
            why.push("symbol name matches the subject".to_string());
        }
        let body_lower: String = shown
            .iter()
            .map(|line| line.line_text.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        let relation_hits = relation_terms
            .iter()
            .filter(|term| body_lower.contains(term.as_str()))
            .count();
        if relation_hits > 0 {
            score += (relation_hits as i32) * 6;
            why.push(format!("relation terms in the region: {relation_hits}"));
        }
        let support_hits = support
            .iter()
            .filter(|term| body_lower.contains(term.as_str()))
            .count();
        if support_hits > 0 {
            score += (support_hits as i32) * 6;
            why.push(format!("support terms in the region: {support_hits}"));
        }
        if is_test_like(&label, relative_lower) {
            score -= 30;
            why.push("test/example penalty".to_string());
        }

        regions.push(Region {
            kind,
            label,
            start_line: start,
            end_line: end,
            score,
            why,
            lines: shown,
        });
    }

    // A symbol whose *name* matches the subject is a candidate even when its
    // body never repeats the name, which is exactly what `relation:defined` is
    // asking for: a declaration whose body is fields or a signature. Without
    // this, kgrep can only find places the name is used, never where it is
    // introduced, and it ranks a test that mentions the name above the type
    // that defines it.
    for item in items {
        let label_lower = item.label.to_ascii_lowercase();
        if !label_matches_subject(&label_lower, subject_lower, subject_tokens) {
            continue;
        }
        if regions
            .iter()
            .any(|region| region.start_line == item.start_line && region.end_line == item.end_line)
        {
            continue;
        }

        let span = item.end_line.saturating_sub(item.start_line) + 1;
        let expand = match full_region {
            FullRegionMode::Always => true,
            FullRegionMode::Never => false,
            FullRegionMode::Auto => span <= REGION_EXPAND_LINES,
        };
        let shown: Vec<LineMatch> = if expand {
            lines
                .iter()
                .enumerate()
                .filter(|(idx, _)| {
                    let line = idx + 1;
                    line >= item.start_line && line <= item.end_line
                })
                .take(REGION_EXPAND_LINES)
                .map(|(idx, line)| LineMatch {
                    line_number: idx + 1,
                    line_text: line.to_string(),
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut score = 40;
        let mut why = vec!["symbol name matches the subject".to_string()];
        if definition_kind(&item.kind) {
            score += match relation {
                Relation::Defined | Relation::Implementation => 60,
                _ => 20,
            };
            why.push(format!("definition ('{}') for this relation", item.kind));
        }
        if is_test_like(&item.label, relative_lower) {
            score -= 30;
            why.push("test/example penalty".to_string());
        }
        regions.push(Region {
            kind: item.kind.clone(),
            label: item.label.clone(),
            start_line: item.start_line,
            end_line: item.end_line,
            score,
            why,
            lines: shown,
        });
    }

    regions
}

/// Kinds that count as introducing a name.
fn definition_kind(kind: &str) -> bool {
    matches!(
        kind,
        "struct" | "enum" | "trait" | "class" | "interface" | "function" | "impl"
    )
}

/// Whether a symbol name refers to the subject.
///
/// Punctuation is stripped from both sides before comparing, because the same
/// name is spelled differently in the two places it appears: a subject is
/// usually written `mcp_call` and the type that defines it is usually
/// `McpCallTool`. Comparing the raw strings finds neither, which is exactly the
/// bug that made a test file outrank the definition it was testing.
fn label_matches_subject(label_lower: &str, subject_lower: &str, tokens: &[String]) -> bool {
    let label = strip_punctuation(label_lower);
    let subject = strip_punctuation(subject_lower);
    label.contains(&subject)
        || tokens.iter().any(|token| {
            let token = strip_punctuation(token);
            !token.is_empty() && label.contains(&token)
        })
}

/// Lowercase alphanumerics only, so `mcp_call` and `McpCallTool` can be compared.
fn strip_punctuation(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn is_test_like(label: &str, relative_lower: &str) -> bool {
    let label = label.to_ascii_lowercase();
    let file = relative_lower.rsplit('/').next().unwrap_or(relative_lower);
    label.contains("test")
        || label.contains("fixture")
        || relative_lower.contains("_test")
        || relative_lower.contains("/tests/")
        // `tests.rs` and `test_thing.rs` are test files even though the shared
        // role inference does not catch them, because its rules look for
        // `/tests/` and `test_`/`_test` only.
        || file.starts_with("test")
        || file.contains("_test.")
}
