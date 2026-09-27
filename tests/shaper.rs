//! The shaper end to end: choose files, match, group, packet.

use kgrep::cli::{GrepArgs, OutlineArgs, ScopeArgs};
use kgrep::model::Budget;
use kgrep::{lexical, outline};
use std::fs;
use std::path::Path;

fn scope(path: &Path) -> ScopeArgs {
    ScopeArgs {
        file_type: None,
        glob: None,
        hidden: false,
        no_ignore: false,
        no_follow: false,
        path: Some(path.display().to_string()),
    }
}

fn grep_args(path: &Path, query: &str, regex: bool) -> GrepArgs {
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

    let packet = lexical::run_grep(
        root,
        &grep_args(root, "auth_status", false),
        Budget::default(),
    )
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

    let packet = lexical::run_grep(root, &grep_args(root, "wanted", false), Budget::default())
        .expect("grep runs");

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

    let packet = lexical::run_grep(root, &grep_args(root, r"auth_\w+", true), Budget::default())
        .expect("grep runs");

    assert_eq!(packet.total_matches, 2);
}

#[test]
fn an_invalid_regex_is_an_error_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.rs", "fn x() {}\n");

    let err = lexical::run_grep(root, &grep_args(root, "(", true), Budget::default())
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
    let packet =
        lexical::run_grep(root, &grep_args(root, "needle", false), budget).expect("grep runs");

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
    let packet = lexical::run_grep(root, &args, Budget::default()).expect("grep runs");

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
    };
    let result = outline::run_outline(root, &args).expect("outline runs");

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
    let packet =
        lexical::run_grep(root, &grep_args(root, "needle", false), budget).expect("grep runs");

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
    let packet =
        lexical::run_grep(root, &grep_args(root, "needle", false), budget).expect("grep runs");

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
    let packet =
        lexical::run_grep(root, &grep_args(root, "needle", false), budget).expect("grep runs");

    assert_eq!(packet.hits.len(), 3);
    assert_eq!(packet.unlisted_files, 0);
    assert_eq!(packet.summarized_hits, 0);
    assert!(!packet.truncated);
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
    };
    let err = outline::run_outline(root, &args).expect_err("missing file should fail");
    assert!(err.contains("file not found"), "got {err}");
    assert!(err.contains("src/session.rs"), "should suggest, got {err}");
}
