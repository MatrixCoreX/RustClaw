use super::{
    git_success_extra, normalize_action, parse_branch_list, parse_git_log_commits,
    parse_git_status_summary, parse_remote_list,
};

#[test]
fn normalizes_git_basic_action_aliases() {
    assert_eq!(normalize_action("branches"), "branch");
    assert_eq!(normalize_action("list-branches"), "branch");
    assert_eq!(normalize_action("get_current_branch"), "current_branch");
    assert_eq!(normalize_action("current-branch-name"), "current_branch");
    assert_eq!(normalize_action("cached_diff"), "diff_cached");
    assert_eq!(normalize_action("changed-file-names"), "changed_files");
}

#[test]
fn parses_git_status_summary_into_machine_fields() {
    let summary = parse_git_status_summary(
        "## main...origin/main [ahead 2, behind 1]\n M Cargo.toml\nA  src/lib.rs\n?? tmp/note.txt\nR  old.rs -> new.rs\n",
    );

    assert_eq!(summary.branch.as_deref(), Some("main"));
    assert_eq!(summary.upstream.as_deref(), Some("origin/main"));
    assert_eq!(summary.ahead, Some(2));
    assert_eq!(summary.behind, Some(1));
    assert!(!summary.clean);
    assert_eq!(summary.changed_count, 4);
    assert_eq!(summary.staged_count, 2);
    assert_eq!(summary.unstaged_count, 1);
    assert_eq!(summary.untracked_count, 1);
    assert_eq!(
        summary.changed_files,
        vec![
            "Cargo.toml".to_string(),
            "src/lib.rs".to_string(),
            "tmp/note.txt".to_string(),
            "new.rs".to_string()
        ]
    );
}

#[test]
fn status_success_extra_exposes_structured_git_state() {
    let extra = git_success_extra(
        "status",
        "status",
        "status",
        0,
        "## main\n M Cargo.toml\n?? tmp/note.txt\n",
        "exit=0\n## main\n M Cargo.toml\n?? tmp/note.txt\n",
        None,
    );

    assert_eq!(
        extra.get("schema_version").and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        extra.get("branch").and_then(|value| value.as_str()),
        Some("main")
    );
    assert_eq!(
        extra.get("worktree_state").and_then(|value| value.as_str()),
        Some("dirty")
    );
    assert_eq!(
        extra.get("changed_count").and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        extra
            .pointer("/field_value/changed_count")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        extra
            .get("changed_files")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn current_branch_and_changed_files_extra_are_structured() {
    let branch = git_success_extra(
        "current_branch",
        "current_branch",
        "rev-parse",
        0,
        "main\n",
        "exit=0\nmain\n",
        None,
    );
    assert_eq!(
        branch
            .pointer("/field_value/current_branch")
            .and_then(|value| value.as_str()),
        Some("main")
    );

    let changed = git_success_extra(
        "changed_files",
        "changed_files",
        "diff",
        0,
        "Cargo.toml\nsrc/main.rs\n",
        "exit=0\nCargo.toml\nsrc/main.rs\n",
        None,
    );
    assert_eq!(
        changed
            .pointer("/field_value/changed_count")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        changed
            .get("changed_files")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.as_str()),
        Some("Cargo.toml")
    );
}

#[test]
fn show_file_at_rev_success_extra_exposes_source_and_content_fields() {
    let mut input_meta = serde_json::Map::new();
    input_meta.insert("target".to_string(), serde_json::json!("HEAD"));
    input_meta.insert("revision".to_string(), serde_json::json!("HEAD"));
    input_meta.insert("path".to_string(), serde_json::json!("README.md"));
    input_meta.insert(
        "source".to_string(),
        serde_json::json!("git_show_file_at_rev"),
    );
    input_meta.insert(
        "source_kind".to_string(),
        serde_json::json!("git_revision_file"),
    );

    let extra = git_success_extra(
        "show_file_at_rev",
        "show_file_at_rev",
        "show",
        0,
        "# RustClaw\n\ncontent",
        "exit=0\n# RustClaw\n\ncontent",
        Some(&input_meta),
    );

    assert_eq!(
        extra.get("source").and_then(|value| value.as_str()),
        Some("git_show_file_at_rev")
    );
    assert_eq!(
        extra.get("path").and_then(|value| value.as_str()),
        Some("README.md")
    );
    assert_eq!(
        extra
            .pointer("/field_value/revision")
            .and_then(|value| value.as_str()),
        Some("HEAD")
    );
    assert_eq!(
        extra
            .pointer("/field_value/content_excerpt")
            .and_then(|value| value.as_str()),
        Some("# RustClaw")
    );
    assert_eq!(
        extra
            .pointer("/field_value/content_line_count")
            .and_then(|value| value.as_u64()),
        Some(3)
    );
}

#[test]
fn parses_git_log_branch_and_remote_structures() {
    let commits = parse_git_log_commits("abc123 First commit\ndef456 Second subject\n");
    assert_eq!(commits.len(), 2);
    assert_eq!(
        commits[0].get("subject").and_then(|value| value.as_str()),
        Some("First commit")
    );

    let branches = parse_branch_list("* main\n  remotes/origin/main\n");
    assert_eq!(branches.len(), 2);
    assert_eq!(
        branches[0].get("current").and_then(|value| value.as_bool()),
        Some(true)
    );

    let remotes = parse_remote_list(
        "origin\thttps://example.com/repo.git (fetch)\norigin\thttps://example.com/repo.git (push)\n",
    );
    assert_eq!(remotes.len(), 2);
    assert_eq!(
        remotes[0].get("direction").and_then(|value| value.as_str()),
        Some("fetch")
    );
}
