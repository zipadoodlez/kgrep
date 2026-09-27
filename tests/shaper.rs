//! The shaper end to end: choose files, match, group, packet.

use kgrep::cli::{FindArgs, FullRegionMode, GrepArgs, OutlineArgs, ScopeArgs, TraceArgs};
use kgrep::model::{Budget, Query, Verb};
use kgrep::{find, lexical, outline, trace};
use std::fs;
use std::path::Path;

fn scope(path: &Path) -> ScopeArgs {
    ScopeArgs {
        file_type: None,
        glob: None,
        hidden: false,
        no_ignore: false,
        follow: false,
        path: Some(path.display().to_string()),
    }
}

fn grep_args(path: &Path, query: &str, regex: bool) -> Query {
    GrepArgs {
        query: query.to_string(),
        scope: scope(path),
        regex,
        json: false,
        paths_only: false,
        max_tokens: None,
        max_hits: None,
        unbounded: false,
    }
    .to_query()
}

fn write(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

#[test]
fn matches_group_under_the_enclosing_symbol() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "src/lib.rs",
        "pub fn auth_status() -> bool {\n    true\n}\n\npub fn unrelated() {}\n",
    );

    let packet = lexical::run_grep(&grep_args(root, "auth_status", false), Budget::default())
        .expect("grep runs");

    assert_eq!(packet.total_files, 1);
    assert_eq!(packet.total_matches, 1);
    let hit = &packet.hits[0];
    assert_eq!(hit.path, "src/lib.rs");
    assert_eq!(hit.language, "rust");
    assert_eq!(hit.role, "implementation");
    assert_eq!(hit.matches[0].line_number, 1);
    assert!(
        hit.groups
            .iter()
            .any(|group| group.kind == "function" && group.label == "auth_status"),
        "the match should be grouped under its function, got {:?}",
        hit.groups
    );
}

#[test]
fn non_matching_files_are_absent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.rs", "pub fn wanted() {}\n");
    write(root, "b.rs", "pub fn other() {}\n");

    let packet =
        lexical::run_grep(&grep_args(root, "wanted", false), Budget::default()).expect("grep runs");

    assert_eq!(packet.total_files, 1);
    assert_eq!(packet.hits[0].path, "a.rs");
}

#[test]
fn regex_mode_matches_patterns() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "a.rs",
        "fn auth_one() {}\nfn auth_two() {}\nfn other() {}\n",
    );

    let packet = lexical::run_grep(&grep_args(root, r"auth_\w+", true), Budget::default())
        .expect("grep runs");

    assert_eq!(packet.total_matches, 2);
}

#[test]
fn an_invalid_regex_is_an_error_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.rs", "fn x() {}\n");

    let err = lexical::run_grep(&grep_args(root, "(", true), Budget::default())
        .expect_err("invalid regex should fail");
    assert!(err.contains("invalid regex"), "unexpected error: {err}");
}

#[test]
fn the_budget_bounds_stored_matches_but_not_the_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let body = (0..50).map(|i| format!("needle {i}\n")).collect::<String>();
    write(root, "a.txt", &body);

    let budget = Budget {
        max_total_matches: 10,
        ..Budget::default()
    };
    let packet = lexical::run_grep(&grep_args(root, "needle", false), budget).expect("grep runs");

    assert_eq!(packet.total_matches, 50, "the count stays exact");
    assert_eq!(
        packet.omitted_matches, 40,
        "the rest is counted, not stored"
    );
    assert!(packet.truncated);
    let stored: usize = packet.hits.iter().map(|hit| hit.matches.len()).sum();
    assert_eq!(stored, 10, "memory is bounded by the budget");
}

#[test]
fn paths_only_stores_no_match_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.rs", "fn needle() {}\n");

    let mut args = grep_args(root, "needle", false);
    args.paths_only = true;
    let packet = lexical::run_grep(&args, Budget::default()).expect("grep runs");

    assert_eq!(packet.total_files, 1);
    assert!(packet.hits[0].matches.is_empty());
    assert_eq!(packet.total_matches, 1);
}

#[test]
fn outline_lists_symbols_for_a_known_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "src/lib.rs",
        "pub struct Thing {}\n\npub fn first() {}\n\npub enum Colour {}\n",
    );

    let args = OutlineArgs {
        file: "src/lib.rs".to_string(),
        scope: scope(root),
        json: false,
        max_items: None,
        context_json: None,
    }
    .to_query();
    let result = outline::run_outline(&args).expect("outline runs");

    let labels: Vec<&str> = result
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    assert!(labels.contains(&"Thing"), "got {labels:?}");
    assert!(labels.contains(&"first"), "got {labels:?}");
    assert!(labels.contains(&"Colour"), "got {labels:?}");
    assert_eq!(result.language, "rust");
}

#[test]
fn the_detail_budget_summarizes_but_keeps_the_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Two files with matches, so the second one has to give up its detail.
    write(root, "a.rs", "fn needle_a() {}\n");
    let body = (0..40).map(|i| format!("needle {i}\n")).collect::<String>();
    write(root, "b.txt", &body);

    let budget = Budget {
        max_detail_tokens: Some(1),
        ..Budget::default()
    };
    let packet = lexical::run_grep(&grep_args(root, "needle", false), budget).expect("grep runs");

    // The count is the truth regardless of what detail survived.
    assert_eq!(packet.total_matches, 41);
    assert_eq!(packet.total_files, 2);
    assert!(
        packet.summarized_hits >= 1,
        "the tail should lose its detail, got {}",
        packet.summarized_hits
    );
    assert!(packet.truncated);
    let summarized = packet.hits.iter().find(|hit| hit.summarized);
    assert!(summarized.is_some(), "one hit should be summarized");
    let summarized = summarized.unwrap();
    assert!(
        !summarized.path.is_empty(),
        "a summarized hit still names its file"
    );
    assert!(
        summarized.matches.is_empty(),
        "and releases the memory its detail held"
    );
    assert!(
        summarized.omitted_matches > 0,
        "while still reporting how much detail there was"
    );
}

#[test]
fn the_hit_cap_bounds_coverage_and_reports_the_truth() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(root, name, "pub fn needle() {}\n");
    }

    let budget = Budget {
        max_hits: Some(2),
        ..Budget::default()
    };
    let packet = lexical::run_grep(&grep_args(root, "needle", false), budget).expect("grep runs");

    assert_eq!(packet.hits.len(), 2, "only two files are listed");
    assert_eq!(packet.total_files, 3, "but the total stays honest");
    assert_eq!(packet.unlisted_files, 1);
    assert!(packet.truncated);
}

#[test]
fn an_unbounded_budget_reports_everything() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(root, name, "pub fn needle() {}\n");
    }

    let budget = Budget {
        max_hits: None,
        max_detail_tokens: None,
        ..Budget::default()
    };
    let packet = lexical::run_grep(&grep_args(root, "needle", false), budget).expect("grep runs");

    assert_eq!(packet.hits.len(), 3);
    assert_eq!(packet.unlisted_files, 0);
    assert_eq!(packet.summarized_hits, 0);
    assert!(!packet.truncated);
}

#[test]
fn find_ranks_by_path_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/auth/session.rs", "pub fn load() {}\n");
    write(root, "src/other.rs", "pub fn load() {}\n");

    let args = FindArgs {
        query_parts: vec!["auth".to_string(), "session".to_string()],
        scope: scope(root),
        max_files: 10,
        json: false,
        paths_only: false,
        debug_score: false,
    }
    .to_query();
    let packet = find::run_find(&args, Budget::default()).expect("find runs");

    assert_eq!(packet.hits.len(), 1, "only the path that says auth/session");
    assert_eq!(packet.hits[0].path, "src/auth/session.rs");
    assert!(packet.hits[0].score > 0);
    assert!(!packet.hits[0].why.is_empty(), "and it explains itself");
}

#[test]
fn find_refuses_a_file_whose_path_says_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // The body is full of the term, but the path says nothing about it. This is
    // the difference between discovery and search.
    write(
        root,
        "src/other.rs",
        "// auth session auth session\npub fn x() {}\n",
    );

    let args = FindArgs {
        query_parts: vec!["auth".to_string(), "session".to_string()],
        scope: scope(root),
        max_files: 10,
        json: false,
        paths_only: false,
        debug_score: false,
    }
    .to_query();
    let packet = find::run_find(&args, Budget::default()).expect("find runs");

    assert!(
        packet.hits.is_empty(),
        "path evidence gates the candidate, got {:?}",
        packet.hits.iter().map(|h| &h.path).collect::<Vec<_>>()
    );
}

#[test]
fn find_reports_the_true_total_when_it_caps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["a_auth.rs", "b_auth.rs", "c_auth.rs"] {
        write(root, &format!("src/{name}"), "pub fn x() {}\n");
    }

    let args = FindArgs {
        query_parts: vec!["auth".to_string()],
        scope: scope(root),
        max_files: 2,
        json: false,
        paths_only: false,
        debug_score: false,
    }
    .to_query();
    let packet = find::run_find(&args, Budget::default()).expect("find runs");

    assert_eq!(packet.hits.len(), 2);
    assert_eq!(packet.total_files, 3, "the total stays honest");
    assert_eq!(packet.unlisted_files, 1);
}

#[test]
fn trace_requires_a_subject_and_a_relation() {
    assert!(trace::parse_query(["relation:defined"]).is_err());
    assert!(trace::parse_query(["subject:x"]).is_err());
    assert!(trace::parse_query(["subject:x", "nonsense:y"]).is_err());

    let query = trace::parse_query([
        "subject:auth_status",
        "relation:rendered",
        "support:ui",
        "kind:code",
        "path:src/tui",
    ])
    .expect("parses");
    assert_eq!(query.subject, "auth_status");
    assert_eq!(query.relation, kgrep::model::Relation::Rendered);
    assert_eq!(query.support, vec!["ui".to_string()]);
    assert_eq!(query.kind.as_deref(), Some("code"));
    assert_eq!(query.path_hint.as_deref(), Some("src/tui"));
}

#[test]
fn trace_finds_a_definition_spelled_differently() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // The subject is written with underscores; the type that defines it is
    // camel case, and its body never repeats the name. Comparing the raw
    // strings finds neither.
    write(
        root,
        "src/mcp.rs",
        "pub struct McpCallInput {\n    server: String,\n    tool: String,\n}\n",
    );
    write(
        root,
        "src/tests.rs",
        // The test function mentions the subject in its body but not in its
        // name, which is what the real case looks like. Naming it after the
        // subject would make its own label match and hand it the same bonus a
        // declaration gets.
        "fn checks_the_surface() {\n    let a = \"mcp_call\";\n    let b = \"mcp_call\";\n}\n",
    );

    let args = trace_args(root, &["subject:mcp_call", "relation:defined"]);
    let packet = trace::run_trace(&args, Budget::default()).expect("trace runs");

    assert_eq!(
        packet.hits[0].path,
        "src/mcp.rs",
        "the definition should outrank the mention, got {:?}",
        packet.hits.iter().map(|h| &h.path).collect::<Vec<_>>()
    );
    let regions = &packet.hits[0].regions;
    assert!(
        regions.iter().any(|region| region.label == "McpCallInput"),
        "the declaration is a region even though its body never names it, got {:?}",
        regions.iter().map(|r| &r.label).collect::<Vec<_>>()
    );
}

#[test]
fn trace_kind_code_excludes_documentation() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "docs/notes.md",
        "# auth_status\n\nauth_status is rendered\n",
    );
    write(root, "src/view.rs", "// auth_status\npub fn draw() {}\n");

    let args = trace_args(
        root,
        &["subject:auth_status", "relation:rendered", "kind:code"],
    );
    let packet = trace::run_trace(&args, Budget::default()).expect("trace runs");

    assert!(
        packet.hits.iter().all(|hit| !hit.path.ends_with(".md")),
        "docs should be filtered by kind:code"
    );
}

#[test]
fn trace_reports_the_true_total_when_it_caps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["a", "b", "c"] {
        write(
            root,
            &format!("src/{name}.rs"),
            "// widget\npub fn draw() {}\n",
        );
    }

    let mut args = trace_args(root, &["subject:widget", "relation:rendered"]);
    if let Verb::Structural { max_files, .. } = &mut args.verb {
        *max_files = 2;
    }
    let packet = trace::run_trace(&args, Budget::default()).expect("trace runs");

    assert_eq!(packet.hits.len(), 2);
    assert_eq!(packet.total_files, 3, "the total stays honest");
    assert_eq!(packet.unlisted_files, 1);
}

#[test]
fn a_symlinked_directory_is_seen_the_same_way_by_both_walkers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/real/target.rs", "// SYMTOKEN\n");
    std::os::unix::fs::symlink(root.join("src/real"), root.join("src/linked")).unwrap();

    let scope = kgrep::scan::SearchScope {
        root,
        file_type: None,
        glob: None,
        hidden: false,
        no_ignore: false,
        follow: false,
    };
    let collected = kgrep::scan::collect_file_entries(&scope);

    let packet = lexical::run_grep(&grep_args(root, "SYMTOKEN", false), Budget::default())
        .expect("grep runs");

    assert_eq!(
        collected.len(),
        packet.hits.len(),
        "the sequential walker ({}) and the parallel one ({}) disagree about a symlinked \
         directory: {:?} versus {:?}",
        collected.len(),
        packet.hits.len(),
        collected
            .iter()
            .map(|e| &e.relative_path)
            .collect::<Vec<_>>(),
        packet.hits.iter().map(|h| &h.path).collect::<Vec<_>>()
    );
}

fn trace_args(root: &Path, terms: &[&str]) -> Query {
    TraceArgs {
        terms: terms.iter().map(|term| term.to_string()).collect(),
        scope: scope(root),
        max_files: 5,
        max_regions: 6,
        full_region: FullRegionMode::Auto,
        json: false,
        paths_only: false,
        debug_plan: false,
        debug_score: false,
        context_json: None,
    }
    .to_query()
    .expect("the DSL parses")
}

#[test]
fn outline_accepts_the_path_grep_printed_for_an_odd_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/real.rs", "// placeholder\n");
    write(root, "src/odd.rs", "pub fn findable() {}\n// ODDTOKEN\n");

    // Give the file a name that is not valid UTF-8, so grep has to print the
    // disambiguated form.
    let odd = root.join("src/odd.rs");
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = odd.as_os_str().to_owned().into_vec();
    bytes.insert(bytes.len() - 3, 0xff);
    let renamed = root.join(std::ffi::OsString::from_vec(bytes));
    std::fs::rename(&odd, &renamed).unwrap();

    let packet = lexical::run_grep(&grep_args(root, "ODDTOKEN", false), Budget::default())
        .expect("grep runs");
    let printed = packet.hits[0].path.clone();
    assert!(
        printed.contains("#raw="),
        "the display path should carry the raw suffix, got {printed:?}"
    );

    // Hand it straight back, which is what an agent does next.
    let args = OutlineArgs {
        file: printed.clone(),
        scope: scope(root),
        json: false,
        max_items: None,
        context_json: None,
    }
    .to_query();
    let outlined = outline::run_outline(&args)
        .unwrap_or_else(|err| panic!("outline should accept {printed:?}: {err}"));
    assert!(
        outlined.items.iter().any(|item| item.label == "findable"),
        "the decoded file should be the one grep found, got {:?}",
        outlined.items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );
}

#[test]
fn a_missing_file_is_an_error_with_a_suggestion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/session.rs", "pub fn x() {}\n");

    let args = OutlineArgs {
        file: "sessio.rs".to_string(),
        scope: scope(root),
        json: false,
        max_items: None,
        context_json: None,
    }
    .to_query();
    let err = outline::run_outline(&args).expect_err("missing file should fail");
    assert!(err.contains("file not found"), "got {err}");
    assert!(err.contains("src/session.rs"), "should suggest, got {err}");
}

/// `ctags` is optional and this feature is one of its callers, so the test skips
/// rather than fails when the binary is absent. The scanner knows Rust,
/// TypeScript, Python and Markdown only, so a Go file is the case that proves
/// the added reach.
#[test]
fn outline_covers_a_language_the_scanner_cannot_parse() {
    let available = std::process::Command::new("ctags")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !available {
        return;
    }

    let source = concat!(
        "package main\n",
        "\n",
        "type Server struct {\n",
        "\tHost string\n",
        "}\n",
        "\n",
        "func (s *Server) Start() error {\n",
        "\treturn nil\n",
        "}\n",
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "sample.go", source);

    let args = OutlineArgs {
        file: "sample.go".to_string(),
        scope: scope(root),
        json: false,
        max_items: None,
        context_json: None,
    }
    .to_query();
    let result = outline::run_outline(&args).expect("outline runs");
    let labels: Vec<&str> = result
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    assert!(labels.contains(&"Server"), "got {labels:?}");
    assert!(labels.contains(&"Start"), "got {labels:?}");
    assert!(
        labels.contains(&"Host"),
        "the struct field should be listed, got {labels:?}"
    );

    // The in-process scanner is what the repo-wide verbs use, and it is the
    // floor: on this file it finds nothing at all.
    let scanner = outline::extract_file_structure(Path::new("sample.go"), "sample.go", source);
    assert!(
        scanner.items.len() < result.items.len(),
        "scanner found {} items, ctags found {}",
        scanner.items.len(),
        result.items.len()
    );
}
