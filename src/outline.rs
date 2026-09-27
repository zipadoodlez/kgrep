//! File structure: what symbols a file declares, and where.
//!
//! Absorbed from agentgrep's `structure.rs`. The extraction is line-based and
//! approximate on purpose: it needs no parser and no resident state, so it
//! costs nothing in memory and works on any file. A tree-sitter driven version
//! replaces the extraction internals later without changing this interface.
//!
//! One caveat that callers must not forget: item end lines are **approximate**.
//! An item ends on the line before the next item begins, not at a real closing
//! brace. Grouping matches under symbols is fine with that. Anything that needs
//! true extents must wait for the parser-backed version.

use crate::cli::OutlineArgs;
use crate::scan::{SearchScope, read_text_file};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StructureItem {
    pub kind: String,
    pub label: String,
    pub start_line: usize,
    pub end_line: usize,
    pub line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStructure {
    pub language: String,
    pub role: String,
    pub items: Vec<StructureItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineResult {
    pub path: String,
    pub language: String,
    pub role: String,
    pub total_lines: usize,
    pub items: Vec<StructureItem>,
    pub omitted_count: usize,
}

pub fn run_outline(root: &Path, args: &OutlineArgs) -> Result<OutlineResult, String> {
    let path = resolve_outline_path(root, &args.file);
    let Some(text) = read_text_file(&path) else {
        return Err(file_not_found_error(root, &args.file, &path));
    };
    let relative = relative_display(root, &path);
    let structure = extract_file_structure(&path, &relative, &text);
    let total_lines = text.lines().count();

    let items = match args.max_items {
        Some(max) => structure
            .items
            .iter()
            .take(max)
            .cloned()
            .collect::<Vec<_>>(),
        None => structure.items.clone(),
    };
    let omitted_count = structure.items.len().saturating_sub(items.len());

    Ok(OutlineResult {
        path: relative,
        language: structure.language,
        role: structure.role,
        total_lines,
        items,
        omitted_count,
    })
}

/// Resolve the outline target: verbatim if it exists, else the unique
/// unambiguous match by file name anywhere under the root.
fn resolve_outline_path(root: &Path, file: &str) -> PathBuf {
    let direct = root.join(file);
    if direct.exists() {
        return direct;
    }
    let candidate = Path::new(file);
    if candidate.is_absolute() && candidate.exists() {
        return candidate.to_path_buf();
    }
    match unique_name_match(root, file) {
        Some(path) => path,
        None => direct,
    }
}

fn unique_name_match(root: &Path, file: &str) -> Option<PathBuf> {
    let scope = SearchScope::new(root);
    let wanted = Path::new(file);
    let mut found: Option<PathBuf> = None;
    for entry in crate::scan::collect_file_entries(&scope) {
        let matches = entry
            .path
            .file_name()
            .is_some_and(|name| Path::new(name) == wanted)
            || entry.relative_path == file;
        if !matches {
            continue;
        }
        if found.is_some() {
            return None; // ambiguous
        }
        found = Some(entry.path);
    }
    found
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn file_not_found_error(root: &Path, requested: &str, resolved: &Path) -> String {
    let mut message = format!(
        "file not found: {requested} (resolved to {})",
        resolved.display()
    );
    let similar = suggest_similar_files(root, requested);
    if !similar.is_empty() {
        message.push_str("\ndid you mean:\n");
        for candidate in similar {
            message.push_str(&format!("  {candidate}\n"));
        }
    }
    message
}

fn suggest_similar_files(root: &Path, requested: &str) -> Vec<String> {
    let stem = Path::new(requested)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if stem.is_empty() {
        return Vec::new();
    }
    let scope = SearchScope::new(root);
    let mut hits = Vec::new();
    for entry in crate::scan::collect_file_entries(&scope) {
        let name = entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name.contains(&stem) {
            hits.push(entry.relative_path);
            if hits.len() >= 5 {
                break;
            }
        }
    }
    hits
}

pub fn extract_file_structure(path: &Path, relative_path: &str, text: &str) -> FileStructure {
    let language = detect_language(path);
    let mut items = match language {
        "rust" => extract_rust(text),
        "typescript" | "javascript" => extract_ts_js(text),
        "python" => extract_python(text),
        "markdown" => extract_markdown(text),
        _ => extract_generic(text),
    };
    finalize_ranges(text, &mut items);
    FileStructure {
        language: language.to_string(),
        role: infer_role(relative_path),
        items,
    }
}

fn detect_language(path: &Path) -> &'static str {
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
}

pub(crate) fn infer_role(relative_path: &str) -> String {
    let path = relative_path.to_ascii_lowercase();
    if path.contains("/tests/") || path.contains("_test") || path.contains("test_") {
        "test".to_string()
    } else if path.contains("/docs/") || path.ends_with(".md") {
        "docs".to_string()
    } else if path.contains("/ui/") || path.contains("/tui/") || path.contains("view") {
        "ui".to_string()
    } else if path.contains("auth") {
        "auth".to_string()
    } else if path.contains("provider") {
        "provider".to_string()
    } else if path.contains("config") {
        "config".to_string()
    } else if path.contains("handler") || path.contains("router") {
        "handler".to_string()
    } else if path.contains("src/") {
        "implementation".to_string()
    } else {
        "generic".to_string()
    }
}

fn extract_rust(text: &str) -> Vec<StructureItem> {
    let mut items = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_number = idx + 1;
        let trimmed = line.trim_start();
        if let Some(label) = parse_rust_keyword_item(trimmed, "fn") {
            items.push(structure_item("function", label, line_number));
            continue;
        }
        if let Some(label) = parse_rust_keyword_item(trimmed, "struct") {
            items.push(structure_item("struct", label, line_number));
            continue;
        }
        if let Some(label) = parse_rust_keyword_item(trimmed, "enum") {
            items.push(structure_item("enum", label, line_number));
            continue;
        }
        if let Some(label) = parse_rust_keyword_item(trimmed, "trait") {
            items.push(structure_item("trait", label, line_number));
            continue;
        }
        if let Some(label) = parse_rust_impl_item(trimmed) {
            items.push(structure_item("impl", label, line_number));
        }
    }
    items
}

fn extract_ts_js(text: &str) -> Vec<StructureItem> {
    let mut items = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_number = idx + 1;
        let mut trimmed = line.trim_start();
        trimmed = strip_keyword_prefix(trimmed, "export")
            .unwrap_or(trimmed)
            .trim_start();
        if let Some(label) = parse_keyword_identifier(trimmed, "function") {
            items.push(structure_item("function", label, line_number));
            continue;
        }
        if let Some(label) = parse_keyword_identifier(trimmed, "class") {
            items.push(structure_item("class", label, line_number));
            continue;
        }
        if let Some(label) = parse_keyword_identifier(trimmed, "interface") {
            items.push(structure_item("interface", label, line_number));
            continue;
        }
        if let Some(label) = parse_ts_arrow_item(trimmed) {
            items.push(structure_item("function", label, line_number));
        }
    }
    items
}

fn extract_python(text: &str) -> Vec<StructureItem> {
    let mut items = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_number = idx + 1;
        let trimmed = line.trim_start();
        if let Some(label) = parse_keyword_identifier(trimmed, "def") {
            items.push(structure_item("function", label, line_number));
            continue;
        }
        if let Some(label) = parse_keyword_identifier(trimmed, "class") {
            items.push(structure_item("class", label, line_number));
        }
    }
    items
}

fn extract_markdown(text: &str) -> Vec<StructureItem> {
    let mut items = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let bytes = line.as_bytes();
        let level = bytes.iter().take_while(|&&byte| byte == b'#').count();
        if level == 0 || bytes.get(level).copied() != Some(b' ') {
            continue;
        }
        let label = line[level + 1..].trim();
        if label.is_empty() {
            continue;
        }
        items.push(StructureItem {
            kind: format!("heading{level}"),
            label: label.to_string(),
            start_line: idx + 1,
            end_line: idx + 1,
            line_count: 1,
        });
    }
    items
}

fn extract_generic(text: &str) -> Vec<StructureItem> {
    let mut items = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if is_generic_section(trimmed) {
            items.push(structure_item("section", trimmed, idx + 1));
        }
    }
    items
}

fn parse_rust_keyword_item<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let mut rest = strip_rust_visibility(line).trim_start();
    if keyword == "fn" {
        rest = strip_keyword_prefix(rest, "async")
            .unwrap_or(rest)
            .trim_start();
    }
    parse_keyword_identifier(rest, keyword)
}

fn parse_rust_impl_item(line: &str) -> Option<&str> {
    let mut rest = line.trim_start().strip_prefix("impl")?;
    if !rest.is_empty() {
        let next = rest.chars().next()?;
        if !next.is_whitespace() && next != '<' {
            return None;
        }
    }
    rest = rest.trim_start();
    if rest.starts_with('<') {
        rest = skip_balanced(rest, '<', '>')?.trim_start();
    }
    take_identifier_like(rest)
}

fn parse_ts_arrow_item(line: &str) -> Option<&str> {
    let rest = ["const", "let", "var"]
        .iter()
        .find_map(|keyword| strip_keyword_prefix(line, keyword))?;
    let (name, rest) = take_identifier(rest.trim_start())?;
    let rest = rest.trim_start();
    if !rest.starts_with('=') || !rest.contains("=>") {
        return None;
    }
    Some(name)
}

fn parse_keyword_identifier<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = strip_keyword_prefix(line, keyword)?;
    let (identifier, _) = take_identifier(rest.trim_start())?;
    Some(identifier)
}

fn strip_rust_visibility(line: &str) -> &str {
    let Some(rest) = line.strip_prefix("pub") else {
        return line;
    };
    if !rest.is_empty() {
        let Some(next) = rest.chars().next() else {
            return rest;
        };
        if !next.is_whitespace() && next != '(' {
            return line;
        }
    }
    let rest = rest.trim_start();
    if rest.starts_with('(') {
        skip_balanced(rest, '(', ')').unwrap_or(rest)
    } else {
        rest
    }
}

fn strip_keyword_prefix<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    if rest.is_empty() {
        return Some(rest);
    }
    let next = rest.chars().next()?;
    if next.is_whitespace() {
        Some(rest)
    } else {
        None
    }
}

fn take_identifier(input: &str) -> Option<(&str, &str)> {
    let mut end = 0;
    for (idx, ch) in input.char_indices() {
        if idx == 0 {
            if !is_identifier_start(ch) {
                return None;
            }
        } else if !is_identifier_continue(ch) {
            end = idx;
            break;
        }
    }
    if end == 0 {
        end = input.len();
    }
    Some((&input[..end], &input[end..]))
}

fn take_identifier_like(input: &str) -> Option<&str> {
    let mut end = 0;
    for (idx, ch) in input.char_indices() {
        if ch.is_whitespace() || ch == '{' {
            end = idx;
            break;
        }
    }
    if end == 0 {
        end = input.len();
    }
    let token = input[..end].trim_end_matches(':').trim_end_matches(',');
    if token.is_empty() { None } else { Some(token) }
}

fn skip_balanced(input: &str, open: char, close: char) -> Option<&str> {
    let mut depth = 0usize;
    for (idx, ch) in input.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                let next_idx = idx + ch.len_utf8();
                return Some(&input[next_idx..]);
            }
        }
    }
    None
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_generic_section(line: &str) -> bool {
    if line.len() < 4 {
        return false;
    }
    let mut chars = line.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    line.chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | ' '))
}

fn structure_item(kind: &str, label: &str, line_number: usize) -> StructureItem {
    StructureItem {
        kind: kind.to_string(),
        label: label.to_string(),
        start_line: line_number,
        end_line: line_number,
        line_count: 1,
    }
}

/// Approximate ending ranges: an item ends the line before the next item
/// begins. See the module note.
fn finalize_ranges(text: &str, items: &mut [StructureItem]) {
    let total_lines = text.lines().count().max(1);
    for idx in 0..items.len() {
        let end = if idx + 1 < items.len() {
            items[idx + 1]
                .start_line
                .saturating_sub(1)
                .max(items[idx].start_line)
        } else {
            total_lines
        };
        items[idx].end_line = end;
        items[idx].line_count = end.saturating_sub(items[idx].start_line) + 1;
    }
}
