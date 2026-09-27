//! Rendering a packet: text for humans, JSON for programs.
//!
//! Absorbed from agentgrep's `render.rs`. Rendering is a pure function of the
//! packet, so it never touches the filesystem and never grows memory beyond the
//! string it builds.

use crate::model::{Hit, Packet, RenderOptions};
use crate::outline::OutlineResult;
use serde::Serialize;

const MAX_RENDERED_MATCH_LINE_CHARS: usize = 240;
const RENDERED_MATCH_PREFIX_CONTEXT_CHARS: usize = 80;
const MAX_NON_CODE_MATCH_LINES_PER_FILE: usize = 3;

// --- grep ------------------------------------------------------------------

pub fn render_grep_text(packet: &Packet) -> String {
    if packet.paths_only {
        return packet
            .hits
            .iter()
            .map(|hit| hit.path.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut lines = vec![
        format!("query: {}", packet.query),
        format!(
            "matches: {} in {} files",
            packet.total_matches, packet.total_files
        ),
    ];

    for hit in &packet.hits {
        render_grep_hit(hit, &packet.query, packet.regex, &mut lines);
    }

    if packet.omitted_matches > 0 {
        lines.push(String::new());
        lines.push(format!(
            "... {} more matches counted but not stored (budget reached)",
            packet.omitted_matches
        ));
    }
    if packet.summarized_hits > 0 {
        lines.push(format!(
            "... {} of {} listed files have no detail; raise --max-tokens or narrow the query",
            packet.summarized_hits,
            packet.hits.len()
        ));
    }
    if packet.unlisted_files > 0 {
        lines.push(format!(
            "... {} more matching files not listed; raise --max-hits or narrow the query",
            packet.unlisted_files
        ));
    }

    lines.join("\n")
}

fn render_grep_hit(hit: &Hit, query: &str, regex: bool, lines: &mut Vec<String>) {
    lines.push(String::new());
    lines.push(hit.path.clone());

    // Detail dropped for the budget: the name, role and true count survive,
    // which is the cheapest and most useful part of the answer.
    if hit.summarized {
        lines.push(format!(
            "  {} match(es) in this file; detail omitted (packet budget)",
            hit.omitted_matches
        ));
        return;
    }

    if hit.total_symbols > 0 {
        lines.push(format!(
            "  symbols: {} total, {} matched, {} other",
            hit.total_symbols,
            hit.matched_symbol_count,
            hit.total_symbols.saturating_sub(hit.matched_symbol_count)
        ));
    } else {
        lines.push("  symbols: no structural items detected".to_string());
    }

    let non_code_cap = non_code_match_cap(&hit.language);
    let mut displayed = 0usize;

    for group in &hit.groups {
        let remaining_file_matches = non_code_cap
            .map(|cap| cap.saturating_sub(displayed))
            .unwrap_or(usize::MAX);
        if remaining_file_matches == 0 {
            break;
        }
        let visible = group
            .match_indices
            .iter()
            .take(remaining_file_matches)
            .filter_map(|idx| hit.matches.get(*idx))
            .collect::<Vec<_>>();
        if visible.is_empty() {
            continue;
        }

        match (group.start_line, group.end_line) {
            (Some(start), Some(end)) => {
                lines.push(format!(
                    "    - {} {} @ {}-{}",
                    group.kind, group.label, start, end
                ));
            }
            _ => lines.push(format!("    - {}", group.label)),
        }
        for line_match in visible {
            let text = compact_rendered_match_line(&line_match.line_text, query, regex);
            lines.push(format!("      - @ {} {}", line_match.line_number, text));
            displayed += 1;
        }
    }

    if non_code_cap.is_some() && hit.matches.len() > displayed {
        lines.push(format!(
            "    - ... {} more non-code matches omitted; narrow path/glob/type or use paths_only for full file list",
            hit.matches.len().saturating_sub(displayed)
        ));
    }

    if !hit.other_symbols.is_empty() {
        let mut summary = hit
            .other_symbols
            .iter()
            .map(|item| {
                format!(
                    "{} {} @ {}-{}",
                    item.kind, item.label, item.start_line, item.end_line
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        if hit.other_symbols_omitted_count > 0 {
            if !summary.is_empty() {
                summary.push_str("; ");
            }
            summary.push_str(&format!("... {} more", hit.other_symbols_omitted_count));
        }
        lines.push(format!("    - other: {summary}"));
    }
}

fn non_code_match_cap(language: &str) -> Option<usize> {
    match language {
        "json" | "yaml" | "markdown" | "text" | "" => Some(MAX_NON_CODE_MATCH_LINES_PER_FILE),
        _ => None,
    }
}

/// Cap a rendered match line, keeping context around the first occurrence of
/// the query so a minified file cannot flood the caller.
pub fn compact_rendered_match_line(line: &str, query: &str, regex: bool) -> String {
    let char_count = line.chars().count();
    if char_count <= MAX_RENDERED_MATCH_LINE_CHARS {
        return line.to_string();
    }

    let match_start_char = if regex || query.is_empty() {
        0
    } else {
        line.find(query)
            .map(|byte| line[..byte].chars().count())
            .unwrap_or(0)
    };
    let start_char = match_start_char.saturating_sub(RENDERED_MATCH_PREFIX_CONTEXT_CHARS);
    let end_char = start_char
        .saturating_add(MAX_RENDERED_MATCH_LINE_CHARS)
        .min(char_count);
    let start_char = end_char
        .saturating_sub(MAX_RENDERED_MATCH_LINE_CHARS)
        .min(start_char);

    let omitted_prefix = start_char;
    let omitted_suffix = char_count.saturating_sub(end_char);
    let snippet: String = line
        .chars()
        .skip(start_char)
        .take(end_char.saturating_sub(start_char))
        .collect();

    match (omitted_prefix > 0, omitted_suffix > 0) {
        (true, true) => format!(
            "…{} … [truncated: {} chars before, {} chars after]",
            snippet, omitted_prefix, omitted_suffix
        ),
        (true, false) => format!("…{} [truncated: {} chars before]", snippet, omitted_prefix),
        (false, true) => format!("{} … [truncated: {} chars after]", snippet, omitted_suffix),
        (false, false) => snippet,
    }
}

// --- JSON ------------------------------------------------------------------

#[derive(Serialize)]
struct PacketJson<'a> {
    query: &'a str,
    regex: bool,
    root: &'a str,
    files: Vec<HitJson<'a>>,
    total_files: usize,
    total_matches: usize,
    /// Matches counted but not stored, when a budget was reached. Absent from
    /// agentgrep's shape; additive and always present here.
    omitted_matches: usize,
    summarized_hits: usize,
    unlisted_files: usize,
    truncated: bool,
}

#[derive(Serialize)]
struct HitJson<'a> {
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_bytes: Option<&'a str>,
    language: &'a str,
    role: &'a str,
    score: i32,
    why: &'a [String],
    matches: &'a [crate::model::LineMatch],
    groups: Vec<GroupJson<'a>>,
    total_symbols: usize,
    matched_symbol_count: usize,
    other_symbols: &'a [crate::outline::StructureItem],
    other_symbols_omitted_count: usize,
    /// True when this hit's match detail was dropped for the packet budget.
    summarized: bool,
}

#[derive(Serialize)]
struct GroupJson<'a> {
    kind: &'a str,
    label: &'a str,
    start_line: Option<usize>,
    end_line: Option<usize>,
    matches: Vec<&'a crate::model::LineMatch>,
}

pub fn grep_json(packet: &Packet) -> serde_json::Value {
    let files = packet
        .hits
        .iter()
        .map(|hit| HitJson {
            path: &hit.path,
            path_bytes: hit.path_bytes.as_deref(),
            language: &hit.language,
            role: &hit.role,
            score: hit.score,
            why: &hit.why,
            matches: &hit.matches,
            groups: hit
                .groups
                .iter()
                .map(|group| GroupJson {
                    kind: &group.kind,
                    label: &group.label,
                    start_line: group.start_line,
                    end_line: group.end_line,
                    matches: group
                        .match_indices
                        .iter()
                        .filter_map(|idx| hit.matches.get(*idx))
                        .collect(),
                })
                .collect(),
            total_symbols: hit.total_symbols,
            matched_symbol_count: hit.matched_symbol_count,
            other_symbols: &hit.other_symbols,
            other_symbols_omitted_count: hit.other_symbols_omitted_count,
            summarized: hit.summarized,
        })
        .collect();

    serde_json::to_value(PacketJson {
        query: &packet.query,
        regex: packet.regex,
        root: &packet.root,
        files,
        total_files: packet.total_files,
        total_matches: packet.total_matches,
        omitted_matches: packet.omitted_matches,
        summarized_hits: packet.summarized_hits,
        unlisted_files: packet.unlisted_files,
        truncated: packet.truncated,
    })
    .expect("packet JSON is always serializable")
}

// --- find ------------------------------------------------------------------

pub fn render_find_text(packet: &Packet, options: &RenderOptions) -> String {
    if packet.paths_only {
        return packet
            .hits
            .iter()
            .map(|hit| hit.path.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut lines = vec![
        format!("query: {}", packet.query),
        format!("files: {} shown", packet.hits.len()),
    ];

    for (idx, hit) in packet.hits.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}", idx + 1, hit.path));
        lines.push(format!("   role: {}  language: {}", hit.role, hit.language));
        if options.debug_score {
            lines.push(format!("   score: {}", hit.score));
        }
        lines.push("   why:".to_string());
        for reason in &hit.why {
            lines.push(format!("     - {reason}"));
        }
        lines.push("   structure:".to_string());
        for item in &hit.other_symbols {
            lines.push(format!(
                "     - {} {} @ {}-{} ({} lines)",
                item.kind, item.label, item.start_line, item.end_line, item.line_count
            ));
        }
        if hit.other_symbols_omitted_count > 0 {
            lines.push(format!(
                "     ... {} more symbols",
                hit.other_symbols_omitted_count
            ));
        }
    }

    if packet.unlisted_files > 0 {
        lines.push(String::new());
        lines.push(format!(
            "... {} more matching files not listed; raise --max-files or narrow the query",
            packet.unlisted_files
        ));
    }

    lines.join("\n")
}

#[derive(Serialize)]
struct FindJson<'a> {
    query: &'a str,
    root: &'a str,
    files: Vec<FindFileJson<'a>>,
    total_files: usize,
    unlisted_files: usize,
}

#[derive(Serialize)]
struct FindFileJson<'a> {
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_bytes: Option<&'a str>,
    role: &'a str,
    language: &'a str,
    score: i32,
    why: &'a [String],
    structure: FindStructureJson<'a>,
}

#[derive(Serialize)]
struct FindStructureJson<'a> {
    items: &'a [crate::outline::StructureItem],
    omitted_count: usize,
}

pub fn find_json(packet: &Packet) -> serde_json::Value {
    let files = packet
        .hits
        .iter()
        .map(|hit| FindFileJson {
            path: &hit.path,
            path_bytes: hit.path_bytes.as_deref(),
            role: &hit.role,
            language: &hit.language,
            score: hit.score,
            why: &hit.why,
            structure: FindStructureJson {
                items: &hit.other_symbols,
                omitted_count: hit.other_symbols_omitted_count,
            },
        })
        .collect();

    serde_json::to_value(FindJson {
        query: &packet.query,
        root: &packet.root,
        files,
        total_files: packet.total_files,
        unlisted_files: packet.unlisted_files,
    })
    .expect("find JSON is always serializable")
}

// --- trace -----------------------------------------------------------------

pub fn render_trace_text(packet: &Packet, options: &RenderOptions) -> String {
    if packet.paths_only {
        return packet
            .hits
            .iter()
            .map(|hit| hit.path.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut lines = vec![
        format!("query: {}", packet.query),
        format!("files: {} shown", packet.hits.len()),
    ];

    for (idx, hit) in packet.hits.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}", idx + 1, hit.path));
        lines.push(format!("   role: {}  language: {}", hit.role, hit.language));
        if options.debug_score {
            lines.push(format!("   score: {}", hit.score));
        }
        lines.push("   why:".to_string());
        for reason in &hit.why {
            lines.push(format!("     - {reason}"));
        }
        for region in &hit.regions {
            lines.push(String::new());
            lines.push(format!(
                "   - {} {} @ {}-{}",
                region.kind, region.label, region.start_line, region.end_line
            ));
            if options.debug_score {
                lines.push(format!("     region score: {}", region.score));
                for reason in &region.why {
                    lines.push(format!("       - {reason}"));
                }
            }
            for line in &region.lines {
                lines.push(format!(
                    "     @ {} {}",
                    line.line_number,
                    compact_rendered_match_line(&line.line_text, &packet.query, false)
                ));
            }
        }
    }

    if packet.unlisted_files > 0 {
        lines.push(String::new());
        lines.push(format!(
            "... {} more matching files not listed; raise --max-files or narrow the query",
            packet.unlisted_files
        ));
    }

    lines.join("\n")
}

#[derive(Serialize)]
struct TraceJson<'a> {
    query: &'a str,
    root: &'a str,
    files: Vec<TraceFileJson<'a>>,
    total_files: usize,
    unlisted_files: usize,
}

#[derive(Serialize)]
struct TraceFileJson<'a> {
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_bytes: Option<&'a str>,
    role: &'a str,
    language: &'a str,
    score: i32,
    why: &'a [String],
    regions: Vec<TraceRegionJson<'a>>,
}

#[derive(Serialize)]
struct TraceRegionJson<'a> {
    kind: &'a str,
    label: &'a str,
    start_line: usize,
    end_line: usize,
    score: i32,
    why: &'a [String],
    lines: &'a [crate::model::LineMatch],
}

pub fn trace_json(packet: &Packet) -> serde_json::Value {
    let files = packet
        .hits
        .iter()
        .map(|hit| TraceFileJson {
            path: &hit.path,
            path_bytes: hit.path_bytes.as_deref(),
            role: &hit.role,
            language: &hit.language,
            score: hit.score,
            why: &hit.why,
            regions: hit
                .regions
                .iter()
                .map(|region| TraceRegionJson {
                    kind: &region.kind,
                    label: &region.label,
                    start_line: region.start_line,
                    end_line: region.end_line,
                    score: region.score,
                    why: &region.why,
                    lines: &region.lines,
                })
                .collect(),
        })
        .collect();

    serde_json::to_value(TraceJson {
        query: &packet.query,
        root: &packet.root,
        files,
        total_files: packet.total_files,
        unlisted_files: packet.unlisted_files,
    })
    .expect("trace JSON is always serializable")
}

// --- outline ---------------------------------------------------------------

pub fn render_outline_text(result: &OutlineResult) -> String {
    let mut lines = vec![
        format!("file: {}", result.path),
        format!("language: {}", result.language),
        format!("role: {}", result.role),
        format!("lines: {}", result.total_lines),
        format!("symbols: {}", result.items.len()),
    ];
    for item in &result.items {
        lines.push(format!(
            "  - {} {} @ {}-{} ({} lines)",
            item.kind, item.label, item.start_line, item.end_line, item.line_count
        ));
    }
    if result.omitted_count > 0 {
        lines.push(format!("  ... {} more symbols", result.omitted_count));
    }
    lines.join("\n")
}

#[derive(Serialize)]
struct OutlineJson<'a> {
    path: &'a str,
    language: &'a str,
    role: &'a str,
    total_lines: usize,
    items: &'a [crate::outline::StructureItem],
    omitted_count: usize,
}

pub fn outline_json(result: &OutlineResult) -> serde_json::Value {
    serde_json::to_value(OutlineJson {
        path: &result.path,
        language: &result.language,
        role: &result.role,
        total_lines: result.total_lines,
        items: &result.items,
        omitted_count: result.omitted_count,
    })
    .expect("outline JSON is always serializable")
}
