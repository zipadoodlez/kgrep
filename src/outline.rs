//! File structure: what symbols a file declares, and where.
//!
//! Two backends, chosen by who is asking.
//!
//! `outline` asks about **one** file. For the four languages the scanner knows,
//! the scanner is used, because it is more precise and costs no subprocess. For
//! anything else, `outline` may pay about 15 ms to run `ctags` and get real
//! reach: roughly a hundred languages instead of four. That is
//! `extract_with_ctags`, and it is what turns a Go, Ruby or Java repository from
//! an outline of nothing into an outline.
//!
//! `grep`, `find` and `trace` sweep a **repository**, where the same 15 ms per
//! file would turn a 30 ms search into half a minute. They stay on
//! `extract_file_structure`, the in-process line-based scanner, which is also
//! the fallback when `ctags` is not installed.
//!
//! One caveat that callers must not forget: item end lines are **approximate**.
//! An item ends on the line before the next item begins, not at a real closing
//! brace. Grouping matches under symbols is fine with that. Anything that needs
//! true extents must wait for a parser-backed version, and `ctags` does not
//! provide them either, since its Rust, Go, Python and TypeScript parsers never
//! emit an end line.

use crate::model::{Query, Verb};
use crate::scan::{SearchScope, read_text_file};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StructureItem {
    pub kind: String,
    pub label: String,
    pub start_line: usize,
    pub end_line: usize,
    pub line_count: usize,
    /// The enclosing item, as `kind:name`, when the extractor knows it. `ctags`
    /// reports this and the line-based scanner does not, so it is `None` on the
    /// scanner path and the ranges then fall back to the old approximation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
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

/// The languages the in-process scanner actually parses. Everything else falls
/// to `ctags`, which is the whole point: without it a Go or Ruby repository gets
/// an outline of nothing.
fn scanner_knows(language: &str) -> bool {
    matches!(
        language,
        "rust" | "typescript" | "javascript" | "python" | "markdown"
    )
}

pub fn run_outline(query: &Query) -> Result<OutlineResult, String> {
    let root = query.root();
    let Verb::Outline { file, max_items } = &query.verb else {
        return Err("outline needs a file".to_string());
    };
    let path = resolve_outline_path(root, file);
    let Some(text) = read_text_file(&path) else {
        return Err(file_not_found_error(root, file, &path));
    };
    let relative = relative_display(root, &path);
    let structure = extract_outline_structure(&path, &relative, &text);
    let total_lines = text.lines().count();

    let items = match max_items {
        Some(max) => structure.items.iter().take(*max).cloned().collect(),
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
///
/// A path carrying our `#raw=<hex>` suffix is decoded first, so a path printed
/// by `grep` or `find` for a name that is not valid UTF-8 can be handed straight
/// back to `outline`. Without this the round-trip an agent naturally performs,
/// search then read, fails for exactly the files whose names are hardest to
/// retype.
fn resolve_outline_path(root: &Path, file: &str) -> PathBuf {
    if let Some(decoded) = decode_raw_suffix(root, file) {
        return decoded;
    }
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

/// Decode a display path carrying the `#raw=<hex>` suffix back to real bytes.
///
/// Returns `None` when there is no suffix, or the hex is malformed, so the
/// caller falls through to ordinary resolution.
fn decode_raw_suffix(root: &Path, file: &str) -> Option<PathBuf> {
    let (_, hex) = file.rsplit_once("#raw=")?;
    let bytes = decode_hex(hex)?;
    if bytes.is_empty() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = root.join(std::ffi::OsStr::from_bytes(&bytes));
        path.exists().then_some(path)
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        None
    }
}

#[cfg(unix)]
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let value = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(value, 16).ok()?);
    }
    Some(out)
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

/// Extract structure for a single file.
///
/// The scanner wins where it applies, because it is more precise and costs no
/// subprocess. `ctags` is the coverage backend for the languages it does not
/// know, so a Go, Ruby or Java repository gets real items where it used to get
/// only match lines.
fn extract_outline_structure(path: &Path, relative_path: &str, text: &str) -> FileStructure {
    let language = detect_language(path);
    let from_ctags = if scanner_knows(language) {
        None
    } else {
        extract_with_ctags(path)
    };
    if let Some(mut items) = from_ctags {
        finalize_ranges(text, &mut items);
        return FileStructure {
            language: language.to_string(),
            role: infer_role(relative_path),
            items,
        };
    }
    extract_file_structure(path, relative_path, text)
}

/// Run `ctags` on one file and read its tags.
///
/// Returns `None` when ctags is missing, refuses the file, or has nothing to
/// say, so the caller falls back to the scanner rather than reporting an empty
/// file. There is no probe: a missing binary fails to spawn, which is the same
/// fallback as one that refuses the file, and testing by doing keeps this to a
/// single exec. `KGREP_CTAGS=off` forces the fallback, which is the control for
/// measuring what ctags actually buys.
fn extract_with_ctags(path: &Path) -> Option<Vec<StructureItem>> {
    if std::env::var("KGREP_CTAGS").as_deref() == Ok("off") {
        return None;
    }
    let output = Command::new("ctags")
        .args(["-f", "-", "--fields=+nKZ", "--sort=no"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut items = parse_ctags_tags(&text);
    if items.is_empty() {
        return None;
    }
    items.sort_by_key(|item| item.start_line);
    Some(items)
}

/// Parse `ctags` tag lines: name, path, pattern, kind, then `key:value` fields.
///
/// The `!_TAG_` header lines are metadata and are skipped. A tag without a line
/// number is dropped, since a symbol we cannot place is not worth reporting.
fn parse_ctags_tags(text: &str) -> Vec<StructureItem> {
    let mut items = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with("!_TAG_") {
            continue;
        }
        // The pattern field is not safe to index positionally: it can contain a
        // literal tab, because some languages indent with tabs, which is exactly
        // what Go does. So the kind is the field after the one that closes the
        // pattern with `;"`.
        let fields: Vec<&str> = line.split('\t').collect();
        let Some(&name) = fields.first() else {
            continue;
        };
        let Some(close) = fields.iter().position(|field| field.ends_with(";\"")) else {
            continue;
        };
        let Some(raw_kind) = fields.get(close + 1) else {
            continue;
        };
        // A `package` declaration is file-level metadata rather than structure:
        // it names the file and encloses everything in it, so keeping it would
        // give one item that swallows the whole outline.
        if *raw_kind == "package" {
            continue;
        }
        let mut line_number = 0usize;
        let mut scope = None;
        for field in fields.iter().skip(close + 2) {
            if let Some(value) = field.strip_prefix("line:") {
                line_number = value.parse().unwrap_or(0);
            } else if let Some(value) = field.strip_prefix("scope:") {
                // `scope:<kind>:<name>`, where the name can itself contain
                // colons, so only the kind is split off.
                if let Some((kind, name)) = value.split_once(':') {
                    scope = Some(format!("{}:{name}", normalize_ctags_kind(kind)));
                }
            }
        }
        if name.is_empty() || line_number == 0 {
            continue;
        }
        items.push(StructureItem {
            kind: normalize_ctags_kind(raw_kind).to_string(),
            label: name.to_string(),
            start_line: line_number,
            end_line: line_number,
            line_count: 1,
            scope,
        });
    }
    items
}

/// `ctags` kind names are per-language, so only the collisions with our own
/// vocabulary are mapped. Printing the real kind beats collapsing everything to
/// "symbol", because the kind is most of what an outline is for.
fn normalize_ctags_kind(raw: &str) -> &str {
    match raw {
        "implementation" => "impl",
        other => other,
    }
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
        // Only used to label a file, so that a Go or Ruby hit reports its real
        // language instead of "text". The scanner still parses only the four
        // above; anything here that it does not know reaches `ctags`.
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "php" => "php",
        "kt" => "kotlin",
        "swift" => "swift",
        "scala" => "scala",
        "sh" | "bash" => "shell",
        "lua" => "lua",
        "sql" => "sql",
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
            scope: None,
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
        scope: None,
    }
}

/// `ctags` qualifies a parent name with its enclosing scopes in some languages.
/// Go reports `main.Server` for a struct whose own tag label is plain `Server`,
/// so containment has to match on the final segment.
fn names_agree(label: &str, scope_name: &str) -> bool {
    label == scope_name || scope_name.ends_with(&format!(".{label}"))
}

/// Approximate ending ranges, containment aware when the extractor reports
/// scopes. An item is bounded by the next item that is not nested inside it, so
/// a struct is not truncated by its own fields. See the module note.
fn finalize_ranges(text: &str, items: &mut [StructureItem]) {
    let total_lines = text.lines().count().max(1);

    // Resolve containment first. `ctags` reports a parent as `kind:name`, but the
    // name repeats: a struct and every impl of it all carry the label
    // `DisplayConfig`. So the parent is the nearest preceding item whose kind
    // and label both match the scope, which is what lets two impls of the same
    // type tell their methods apart.
    let mut parents: Vec<Option<usize>> = vec![None; items.len()];
    for idx in 0..items.len() {
        let Some(scope) = items[idx].scope.as_deref() else {
            continue;
        };
        let Some((kind, name)) = scope.split_once(':') else {
            continue;
        };
        parents[idx] = (0..idx)
            .rev()
            .find(|prev| items[*prev].kind == kind && names_agree(&items[*prev].label, name));
    }

    fn descends(parents: &[Option<usize>], mut child: usize, ancestor: usize) -> bool {
        while let Some(parent) = parents[child] {
            if parent == ancestor {
                return true;
            }
            child = parent;
        }
        false
    }

    for idx in 0..items.len() {
        // An item declared inside another does not end it, so a struct is not
        // truncated to its first line by its own fields. Only the next item that
        // is not a descendant bounds the extent.
        let end = items
            .iter()
            .enumerate()
            .skip(idx + 1)
            .find(|(later, _)| !descends(&parents, *later, idx))
            .map(|(_, later)| later.start_line.saturating_sub(1))
            .unwrap_or(total_lines)
            .max(items[idx].start_line);
        items[idx].end_line = end;
        items[idx].line_count = end.saturating_sub(items[idx].start_line) + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, label: &str, start_line: usize, scope: Option<&str>) -> StructureItem {
        StructureItem {
            kind: kind.to_string(),
            label: label.to_string(),
            start_line,
            end_line: start_line,
            line_count: 1,
            scope: scope.map(|s| s.to_string()),
        }
    }

    /// The tag lines below are the real shape `ctags` emits with
    /// `--fields=+nKZ`: name, path, pattern, kind, then `key:value` fields.
    #[test]
    fn parses_ctags_lines_into_items_with_scope() {
        let tags = concat!(
            "!_TAG_FILE_FORMAT\t2\t/extended format/\n",
            "DisplayConfig\tcrates/x/display.rs\t/^pub struct DisplayConfig {$/;\"\tstruct\tline:12\n",
            "diff_mode\tcrates/x/display.rs\t/^    pub diff_mode: bool,$/;\"\tfield\tline:15\tscope:struct:DisplayConfig\n",
            "set_default\tcrates/x/display.rs\t/^    fn set_default() {$/;\"\tmethod\tline:180\tscope:implementation:DisplayConfig\n",
            "unplaceable\tcrates/x/display.rs\t/^whatever$/;\"\tfield\n",
        );
        let items = parse_ctags_tags(tags);

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["DisplayConfig", "diff_mode", "set_default"]);
        assert_eq!(items[0].kind, "struct");
        assert_eq!(items[0].start_line, 12);
        assert_eq!(items[0].scope, None);
        assert_eq!(items[1].kind, "field");
        assert_eq!(items[1].scope.as_deref(), Some("struct:DisplayConfig"));
        // A scope kind is normalized to match our own vocabulary, and a tag with
        // no line number is dropped rather than reported at line zero.
        assert_eq!(items[2].kind, "method");
        assert_eq!(items[2].scope.as_deref(), Some("impl:DisplayConfig"));
    }

    /// Go indents with tabs and `ctags` puts the raw line in the pattern field,
    /// so the pattern contains a literal tab. Indexing fields positionally reads
    /// the kind as the rest of the pattern, which silently mislabels the item.
    #[test]
    fn a_tab_inside_the_pattern_does_not_shift_the_kind() {
        let tags = "Host\tcrates/x/sample.go\t/^\tHost string$/;\"\tmember\tline:4\tscope:struct:main.Server\n";
        let items = parse_ctags_tags(tags);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Host");
        assert_eq!(items[0].kind, "member");
        assert_eq!(items[0].start_line, 4);
        assert_eq!(items[0].scope.as_deref(), Some("struct:main.Server"));
    }

    #[test]
    fn a_parent_is_not_truncated_by_its_own_members() {
        let mut items = vec![
            item("struct", "A", 1, None),
            item("field", "f1", 3, Some("struct:A")),
            item("field", "f2", 5, Some("struct:A")),
            item("impl", "A", 8, None),
            item("method", "m", 9, Some("impl:A")),
        ];
        let text = "x\n".repeat(20);
        finalize_ranges(&text, &mut items);

        // The struct runs to the line before the next sibling, not to the line
        // before its first field.
        assert_eq!(items[0].end_line, 7, "struct should not be cut by a field");
        assert_eq!(items[0].line_count, 7);
        // The impl covers its method and runs to the end of the file.
        assert_eq!(items[3].end_line, 20, "impl should swallow its method");
        // Two impls of the same type are told apart by kind, so the method at 9
        // attaches to the impl at 8 rather than to the struct at 1.
        assert_eq!(items[4].scope.as_deref(), Some("impl:A"));
    }

    /// Go qualifies a parent with the package: it reports `main.Server` for a
    /// struct whose own label is plain `Server`. Matching only on the full name
    /// would leave the struct 1 line long with its member floating outside it.
    #[test]
    fn a_qualified_scope_name_still_resolves_to_its_parent() {
        let mut items = vec![
            item("struct", "Server", 3, None),
            item("member", "Host", 4, Some("struct:main.Server")),
            item("func", "Later", 9, None),
        ];
        let text = "x\n".repeat(12);
        finalize_ranges(&text, &mut items);
        assert_eq!(items[0].end_line, 8, "the struct should contain its member");
    }

    #[test]
    fn the_scanner_path_keeps_the_old_approximation() {
        let mut items = vec![
            item("function", "a", 1, None),
            item("function", "b", 9, None),
        ];
        let text = "x\n".repeat(12);
        finalize_ranges(&text, &mut items);
        assert_eq!(items[0].end_line, 8);
        assert_eq!(items[1].end_line, 12);
    }
}
