//! Choosing and reading candidate files.
//!
//! Absorbed from agentgrep's `workspace.rs`. This is the streaming front of the
//! funnel: it yields file entries one walk at a time and never holds file
//! contents, so memory here is bounded by the walk, not by the corpus.
//!
//! Non-UTF-8 file names are kept addressable. The display path of such a name
//! gets a `#raw=<hex>` suffix over the full relative path bytes, which is
//! injective: two distinct native names can never render the same, so a
//! consumer that dedups on the displayed path cannot silently drop a file.
//! This differs from agentgrep's more elaborate per-byte token scheme; the
//! differential test compares UTF-8 corpora first.

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
    /// Follow symlinks during the walk. Defaults to true.
    pub follow: bool,
}

impl<'a> SearchScope<'a> {
    /// A plain scope: root only, default ignore rules, symlinks followed.
    pub fn new(root: &'a Path) -> Self {
        Self {
            root,
            file_type: None,
            glob: None,
            hidden: false,
            no_ignore: false,
            follow: true,
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

pub fn collect_file_entries(scope: &SearchScope<'_>) -> Vec<FileEntry> {
    let mut builder = WalkBuilder::new(scope.root);
    builder.hidden(!scope.hidden);
    builder.follow_links(scope.follow);
    if scope.no_ignore {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
        builder.ignore(false);
    } else {
        builder.add_custom_ignore_filename(".rgignore");
    }

    let file_type = scope.file_type.map(normalize_file_type);
    let glob = scope.glob.and_then(|g| build_glob(scope.root, g));
    let mut files = Vec::new();

    for entry in builder.build() {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if !scope.follow && entry.path_is_symlink() {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if let Some(expected_ext) = file_type.as_deref()
            && path.extension().and_then(|s| s.to_str()) != Some(expected_ext)
        {
            continue;
        }
        if let Some(glob) = &glob
            && glob.matched(path, false).is_ignore()
        {
            continue;
        }
        let relative_path = normalize_display_path(scope.root, path);
        let relative_raw = relative_raw_bytes(scope.root, path);
        files.push(FileEntry {
            path: path.to_path_buf(),
            relative_path,
            relative_raw,
        });
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
