//! Choosing and reading candidate files.
//!
//! Absorbed from agentgrep's `workspace.rs`, then reshaped so that the walk can
//! be **streamed** instead of collected. Three things live here and nowhere
//! else, because a second copy of any of them is how the two searchers drifted
//! apart in agentgrep:
//!
//! * `walker_for` — the one place walker configuration is set.
//! * `ScanConfig` — the file-type and glob filter, and what makes an entry
//!   admissible, shared by the streaming and collected paths.
//! * `read_text_file` — the binary and encoding rules.
//!
//! Non-UTF-8 file names are kept addressable. The display path of such a name
//! gets a `#raw=<hex>` suffix over the full relative path bytes, which is
//! injective: two distinct native names can never render the same. This differs
//! from agentgrep's per-byte token scheme.

use crate::model::Where;
use ignore::WalkBuilder;
use ignore::overrides::{Override, OverrideBuilder};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SearchScope<'a> {
    pub root: &'a Path,
    pub file_type: Option<&'a str>,
    pub glob: Option<&'a str>,
    pub hidden: bool,
    pub no_ignore: bool,
    /// Follow symlinks during the walk. Off by default; see `cli::ScopeArgs`.
    pub follow: bool,
}

impl<'a> SearchScope<'a> {
    /// Narrow a search to everything a query's `Where` asks for.
    ///
    /// One home for the mapping, because all four verbs narrow identically and
    /// four copies of it is four places to forget a field.
    pub fn from_where(where_: &'a Where) -> Self {
        Self {
            root: &where_.root,
            file_type: where_.file_type.as_deref(),
            glob: where_.glob.as_deref(),
            hidden: where_.hidden,
            no_ignore: where_.no_ignore,
            follow: where_.follow,
        }
    }

    /// A plain scope: root only, default ignore rules, symlinks not followed.
    pub fn new(root: &'a Path) -> Self {
        Self {
            root,
            file_type: None,
            glob: None,
            hidden: false,
            no_ignore: false,
            follow: false,
        }
    }
}

/// The one place walker configuration is decided.
pub fn walker_for(scope: &SearchScope<'_>) -> WalkBuilder {
    let mut builder = WalkBuilder::new(scope.root);
    builder.hidden(!scope.hidden);
    // Follow symlinks so directly-symlinked files and directories are searched.
    builder.follow_links(scope.follow);
    if scope.no_ignore {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
        builder.ignore(false);
    } else {
        // ripgrep honors `.rgignore` by default; the ignore crate only knows
        // `.ignore` and `.gitignore`, so register it explicitly.
        builder.add_custom_ignore_filename(".rgignore");
    }
    builder
}

/// The file-type and glob filter, owned so it can move into a walker closure.
#[derive(Debug, Clone)]
pub struct ScanConfig {
    root: PathBuf,
    filter: ScopeFilter,
    follow: bool,
    hidden: bool,
    no_ignore: bool,
}

#[derive(Debug, Clone)]
struct ScopeFilter {
    extension: Option<String>,
    glob: Option<Override>,
}

impl ScanConfig {
    pub fn new(scope: &SearchScope<'_>) -> Self {
        Self {
            root: scope.root.to_path_buf(),
            filter: ScopeFilter {
                extension: scope.file_type.map(normalize_file_type),
                glob: scope.glob.and_then(|glob| build_glob(scope.root, glob)),
            },
            follow: scope.follow,
            hidden: scope.hidden,
            no_ignore: scope.no_ignore,
        }
    }

    /// A walker for this configuration, so a closure does not have to borrow a
    /// `SearchScope` with its lifetime.
    pub fn walker(&self) -> WalkBuilder {
        let scope = SearchScope {
            root: &self.root,
            file_type: None,
            glob: None,
            hidden: self.hidden,
            no_ignore: self.no_ignore,
            follow: self.follow,
        };
        walker_for(&scope)
    }

    /// Whether an entry is admissible. Mirrors the collected path exactly.
    pub fn accepts(&self, path: &Path) -> bool {
        // `is_file()` follows a symlink, so a link to a file passes and a link
        // to a directory does not. That is exactly agentgrep v0.1.6's
        // behaviour, which never set `follow_links` and had no symlink guard:
        // linked files are searched, linked directories are not descended.
        // `--follow` adds the directories on top.
        if !path.is_file() {
            return false;
        }
        if let Some(expected) = self.filter.extension.as_deref()
            && path.extension().and_then(|s| s.to_str()) != Some(expected)
        {
            return false;
        }
        if let Some(glob) = &self.filter.glob
            && glob.matched(path, false).is_ignore()
        {
            return false;
        }
        true
    }

    pub fn entry(&self, path: &Path) -> FileEntry {
        FileEntry {
            path: path.to_path_buf(),
            relative_path: normalize_display_path(&self.root, path),
            relative_raw: relative_raw_bytes(&self.root, path),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    /// Lossy relative path used for matching, scoring, and role inference.
    pub relative_path: String,
    /// Raw bytes of the relative path when it is not valid UTF-8 (Unix only).
    pub relative_raw: Option<Vec<u8>>,
}

impl FileEntry {
    /// Display path that is unique per file. See the module note.
    pub fn display_path(&self) -> String {
        match &self.relative_raw {
            Some(raw) => format!("{}#raw={}", self.relative_path, path_bytes_hex(raw)),
            None => self.relative_path.clone(),
        }
    }

    /// Full hex encoding of the raw relative path bytes for JSON consumers,
    /// present only when the path is not valid UTF-8.
    pub fn path_bytes_hex(&self) -> Option<String> {
        self.relative_raw.as_deref().map(path_bytes_hex)
    }
}

/// Collect every candidate. Only the outline verb needs this; the search streams.
pub fn collect_file_entries(scope: &SearchScope<'_>) -> Vec<FileEntry> {
    let config = ScanConfig::new(scope);
    let mut files = Vec::new();
    for entry in walker_for(scope).build() {
        let Ok(entry) = entry else {
            continue;
        };
        if !config.accepts(entry.path()) {
            continue;
        }
        files.push(config.entry(entry.path()));
    }
    // Deterministic ordering, independent of filesystem readdir order.
    files.sort_by(|a, b| a.path.as_os_str().cmp(b.path.as_os_str()));
    files
}

/// Read a file as text, refusing binaries.
///
/// A NUL byte marks a binary file, and invalid UTF-8 is decoded lossily rather
/// than dropped, so a stray byte in a comment does not hide a whole file.
pub fn read_text_file(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(err) => Some(String::from_utf8_lossy(err.as_bytes()).into_owned()),
    }
}

fn build_glob(root: &Path, glob: &str) -> Option<Override> {
    let mut builder = OverrideBuilder::new(root);
    builder.add(glob).ok()?;
    builder.build().ok()
}

pub fn normalize_file_type(file_type: &str) -> String {
    match file_type {
        "rust" => "rs".to_string(),
        "javascript" => "js".to_string(),
        "typescript" => "ts".to_string(),
        other => other.trim_start_matches('.').to_string(),
    }
}

pub fn normalize_display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Raw bytes of the root-relative path when it is not valid UTF-8.
pub fn relative_raw_bytes(root: &Path, path: &Path) -> Option<Vec<u8>> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative.to_str().is_some() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(relative.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Hex encoding of raw path bytes, e.g. `61ff2e747874` for `b"a\xff.txt"`.
pub fn path_bytes_hex(raw: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(raw.len() * 2);
    for byte in raw {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
