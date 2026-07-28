use std::path::PathBuf;
use std::time::Duration;

use crate::{
    discover, ripgrep_status, ripgrep_text_search, BackendPreference, CancellationToken, CaseMode,
    Completeness, DiscoveryBackend, DiscoveryRequest, MatchMode, RipgrepTextRequest, TargetKind,
    TextPatternKind,
};

fn fixture_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rustclaw-fs-discovery-{name}-{}",
        std::process::id()
    ))
}

fn clean_fixture(path: &PathBuf) {
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn default_search_reaches_beyond_eight_levels() {
    let root = fixture_root("deep");
    clean_fixture(&root);
    let mut nested = root.clone();
    for depth in 0..12 {
        nested.push(format!("level-{depth}"));
    }
    std::fs::create_dir_all(&nested).expect("create deep fixture");
    std::fs::write(nested.join("needle.txt"), "needle\n").expect("write target");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.selector.patterns = vec!["needle.txt".to_string()];
    request.selector.match_mode = MatchMode::Exact;
    request.selector.target_kind = TargetKind::File;
    let report = discover(&request).expect("deep search");

    assert_eq!(report.completeness, Completeness::Complete);
    assert_eq!(report.entries.len(), 1);
    assert!(report.entries[0].relative_path.ends_with("needle.txt"));
    clean_fixture(&root);
}

#[test]
fn ignore_and_hidden_defaults_follow_codex_style_discovery() {
    let root = fixture_root("ignore-hidden");
    clean_fixture(&root);
    std::fs::create_dir_all(root.join("ignored")).expect("create ignored");
    std::fs::create_dir_all(root.join(".hidden")).expect("create hidden");
    std::fs::create_dir_all(root.join("ignored-by-dotignore")).expect("create ignored by .ignore");
    std::fs::write(root.join(".gitignore"), "ignored/\n").expect("write gitignore");
    std::fs::write(root.join(".ignore"), "ignored-by-dotignore/\n").expect("write .ignore");
    std::fs::write(root.join("ignored/needle.txt"), "ignored\n").expect("write ignored");
    std::fs::write(root.join("ignored-by-dotignore/needle.txt"), "ignored\n")
        .expect("write dotignore target");
    std::fs::write(root.join(".hidden/needle.txt"), "hidden\n").expect("write hidden");
    std::fs::write(root.join("needle.txt"), "visible\n").expect("write visible");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.selector.patterns = vec!["needle.txt".to_string()];
    request.selector.match_mode = MatchMode::Exact;
    request.selector.target_kind = TargetKind::File;
    let normal = discover(&request).expect("default search");
    assert_eq!(normal.entries.len(), 1);
    assert_eq!(normal.entries[0].relative_path, "needle.txt");

    request.policy.include_hidden = true;
    request.policy.respect_ignore = false;
    let inclusive = discover(&request).expect("inclusive search");
    assert_eq!(inclusive.entries.len(), 4);
    clean_fixture(&root);
}

#[test]
fn exact_name_matching_normalizes_case_and_fullwidth_punctuation() {
    let root = fixture_root("normalized-name");
    clean_fixture(&root);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("Release-Notes.md"), "fixture\n").expect("write fixture");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.selector.patterns = vec!["RELEASE－NOTES。MD".to_string()];
    request.selector.match_mode = MatchMode::Exact;
    request.selector.target_kind = TargetKind::File;
    let report = discover(&request).expect("normalized exact search");

    assert_eq!(report.completeness, Completeness::Complete);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].relative_path, "Release-Notes.md");
    clean_fixture(&root);
}

#[test]
fn explicit_depth_is_semantic_scope_not_a_default() {
    let root = fixture_root("depth");
    clean_fixture(&root);
    std::fs::create_dir_all(root.join("one/two")).expect("create nested");
    std::fs::write(root.join("one/two/needle.txt"), "needle\n").expect("write target");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.selector.patterns = vec!["needle.txt".to_string()];
    request.selector.match_mode = MatchMode::Exact;
    request.selector.target_kind = TargetKind::File;
    request.budget.max_depth = Some(1);
    let shallow = discover(&request).expect("shallow search");
    assert!(shallow.entries.is_empty());
    assert_eq!(shallow.completeness, Completeness::Complete);
    clean_fixture(&root);
}

#[test]
fn hard_limit_and_deadline_are_machine_visible() {
    let root = fixture_root("limits");
    clean_fixture(&root);
    std::fs::create_dir_all(&root).expect("create root");
    for index in 0..20 {
        std::fs::write(root.join(format!("item-{index}.txt")), "fixture\n").expect("write fixture");
    }

    let mut hard = DiscoveryRequest::new(&root, ".");
    hard.budget.hard_entry_limit = 2;
    assert_eq!(
        discover(&hard).expect("hard limited").completeness,
        Completeness::PartialHardLimit
    );

    let mut deadline = DiscoveryRequest::new(&root, ".");
    deadline.budget.deadline = Some(Duration::ZERO);
    assert_eq!(
        discover(&deadline).expect("deadline limited").completeness,
        Completeness::PartialDeadline
    );

    let token = CancellationToken::default();
    token.cancel();
    let mut cancelled = DiscoveryRequest::new(&root, ".");
    cancelled.budget.cancellation = Some(token);
    let cancelled_report = discover(&cancelled).expect("cancelled search");
    assert_eq!(cancelled_report.completeness, Completeness::PartialDeadline);
    assert!(cancelled_report.cancelled);
    clean_fixture(&root);
}

#[test]
fn directory_root_itself_can_match() {
    let workspace = fixture_root("root-match-workspace");
    clean_fixture(&workspace);
    let root = workspace.join("photos");
    std::fs::create_dir_all(&root).expect("create matching root");
    let mut request = DiscoveryRequest::new(&workspace, "photos");
    request.selector.patterns = vec!["photos".to_string()];
    request.selector.match_mode = MatchMode::Exact;
    request.selector.target_kind = TargetKind::Directory;

    let report = discover(&request).expect("root match");
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].relative_path, "photos");
    clean_fixture(&workspace);
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_never_followed() {
    use std::os::unix::fs::symlink;

    let root = fixture_root("symlink-root");
    let outside = fixture_root("symlink-outside");
    clean_fixture(&root);
    clean_fixture(&outside);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&outside).expect("create outside");
    std::fs::write(outside.join("secret.txt"), "secret\n").expect("write secret");
    symlink(&outside, root.join("linked")).expect("create link");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.selector.patterns = vec!["secret.txt".to_string()];
    request.selector.match_mode = MatchMode::Exact;
    request.selector.target_kind = TargetKind::File;
    let report = discover(&request).expect("symlink safe search");
    assert!(report.entries.is_empty());
    assert!(report.skipped_symlinks >= 1);
    clean_fixture(&root);
    clean_fixture(&outside);
}

#[cfg(unix)]
#[test]
fn symlink_search_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let workspace = fixture_root("symlink-explicit-root");
    clean_fixture(&workspace);
    std::fs::create_dir_all(workspace.join("real")).expect("create real root");
    symlink(workspace.join("real"), workspace.join("linked")).expect("create root link");
    let request = DiscoveryRequest::new(&workspace, "linked");
    let error = discover(&request).expect_err("symlink root must fail");
    assert_eq!(error.code(), "unsupported_root");
    clean_fixture(&workspace);
}

#[cfg(unix)]
#[test]
fn unreadable_directory_is_reported_as_partial_permission() {
    use std::os::unix::fs::PermissionsExt;

    let root = fixture_root("permission");
    clean_fixture(&root);
    let denied = root.join("denied");
    std::fs::create_dir_all(&denied).expect("create denied directory");
    std::fs::write(root.join("visible.txt"), "visible\n").expect("write visible fixture");
    std::fs::write(denied.join("secret.txt"), "secret\n").expect("write denied fixture");
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000))
        .expect("remove directory permissions");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.selector.target_kind = TargetKind::File;
    let report = discover(&request).expect("permission-aware search");

    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o700))
        .expect("restore directory permissions");
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.relative_path == "visible.txt"));
    if report.permission_denied > 0 {
        assert_eq!(report.completeness, Completeness::PartialPermission);
        assert!(!report
            .entries
            .iter()
            .any(|entry| entry.relative_path.ends_with("secret.txt")));
    } else {
        // Privileged test runners can traverse mode-000 directories. In that
        // environment the fixture cannot manufacture PermissionDenied, but it
        // must still remain a complete and deterministic scan.
        assert_eq!(report.completeness, Completeness::Complete);
    }
    clean_fixture(&root);
}

#[test]
fn outside_workspace_root_fails_closed() {
    let root = fixture_root("workspace");
    let outside = fixture_root("outside");
    clean_fixture(&root);
    clean_fixture(&outside);
    std::fs::create_dir_all(&root).expect("create workspace");
    std::fs::create_dir_all(&outside).expect("create outside");

    let request = DiscoveryRequest::new(&root, &outside);
    let error = discover(&request).expect_err("outside path must fail");
    assert_eq!(error.code(), "outside_workspace");
    clean_fixture(&root);
    clean_fixture(&outside);
}

#[test]
fn rust_backend_supports_path_globs_and_smart_case() {
    let root = fixture_root("glob-smart-case");
    clean_fixture(&root);
    std::fs::create_dir_all(root.join("crate/src")).expect("create source dir");
    std::fs::create_dir_all(root.join("crate/tests")).expect("create test dir");
    std::fs::write(root.join("crate/src/Widget.RS"), "").expect("write source");
    std::fs::write(root.join("crate/tests/widget.rs"), "").expect("write test");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.backend = BackendPreference::Rust;
    request.selector.target_kind = TargetKind::File;
    request.selector.case_mode = CaseMode::Smart;
    request.selector.globs = vec!["**/src/*.rs".to_string()];
    let report = discover(&request).expect("glob search");

    assert_eq!(report.backend.backend, DiscoveryBackend::Rust);
    assert_eq!(report.entries.len(), 1);
    assert!(report.entries[0]
        .relative_path
        .ends_with("crate/src/Widget.RS"));
    clean_fixture(&root);
}

#[test]
fn rust_and_ripgrep_file_backends_have_normalized_result_parity() {
    if !ripgrep_status().available {
        return;
    }
    let root = fixture_root("backend-parity");
    clean_fixture(&root);
    std::fs::create_dir_all(root.join(".git")).expect("create git marker");
    std::fs::create_dir_all(root.join("ignored")).expect("create ignored dir");
    std::fs::create_dir_all(root.join("src/nested")).expect("create source dir");
    std::fs::write(root.join(".gitignore"), "ignored/\n").expect("write ignore file");
    std::fs::write(root.join("ignored/skip.rs"), "").expect("write ignored file");
    std::fs::write(root.join("src/lib.rs"), "").expect("write source");
    std::fs::write(root.join("src/nested/mod.rs"), "").expect("write nested source");

    let mut request = DiscoveryRequest::new(&root, ".");
    request.selector.target_kind = TargetKind::File;
    request.selector.extensions = vec!["rs".to_string()];
    request.backend = BackendPreference::Rust;
    let rust = discover(&request).expect("Rust backend");
    request.backend = BackendPreference::Ripgrep;
    let ripgrep = discover(&request).expect("ripgrep backend");
    let rust_paths = rust
        .entries
        .iter()
        .map(|entry| entry.relative_path.as_str())
        .collect::<Vec<_>>();
    let ripgrep_paths = ripgrep
        .entries
        .iter()
        .map(|entry| entry.relative_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(rust_paths, ripgrep_paths);
    assert_eq!(rust_paths.len(), 2);
    assert!(ripgrep_paths.iter().all(|path| !path.starts_with("./")));
    assert_eq!(ripgrep.backend.backend, DiscoveryBackend::Ripgrep);
    clean_fixture(&root);
}

#[test]
fn ripgrep_json_text_backend_returns_exact_literal_spans() {
    if !ripgrep_status().available {
        return;
    }
    let root = fixture_root("text-spans");
    clean_fixture(&root);
    std::fs::create_dir_all(&root).expect("create root");
    let file = root.join("sample.txt");
    std::fs::write(&file, "alpha\nneedle needle\n").expect("write text fixture");
    let report = ripgrep_text_search(&RipgrepTextRequest {
        workspace_root: root.clone(),
        root: root.clone(),
        paths: vec![file],
        query: "needle".to_string(),
        pattern_kind: TextPatternKind::Literal,
        case_mode: CaseMode::Sensitive,
        multiline: false,
        max_matches: 10,
        max_output_bytes: 64 * 1024,
        max_line_chars: 240,
        deadline: Some(Duration::from_secs(5)),
        cancellation: None,
    })
    .expect("ripgrep JSON text search");

    assert_eq!(report.completeness, Completeness::Complete);
    assert_eq!(report.matches.len(), 2);
    assert_eq!(
        (report.matches[0].start_byte, report.matches[0].end_byte),
        (6, 12)
    );
    assert_eq!(
        (report.matches[1].start_byte, report.matches[1].end_byte),
        (13, 19)
    );
    assert_eq!(report.matches[0].line, 2);
    clean_fixture(&root);
}

#[cfg(unix)]
#[test]
fn bounded_child_deadline_terminates_the_process_group() {
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg("sleep 30 & wait");
    let budget = crate::DiscoveryBudget {
        deadline: Some(Duration::from_millis(30)),
        ..crate::DiscoveryBudget::default()
    };
    let started = std::time::Instant::now();
    let captured = crate::ripgrep_process::run_bounded(command, &budget, 4096, false)
        .expect("deadline termination");

    assert!(captured.timed_out);
    assert!(started.elapsed() < Duration::from_secs(3));
}
