use super::{
    error_extra, execute_with_workspace_root, git_success_extra, normalize_action,
    parse_branch_list, parse_git_log_commits, parse_git_status_summary, parse_remote_list,
    SKILL_NAME,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEST_REPO_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRepository {
    workspace: PathBuf,
    repository: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let sequence = TEST_REPO_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "rustclaw-git-basic-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        let repository = workspace.join("repository");
        std::fs::create_dir_all(&repository).expect("create test repository");
        run_git(&repository, &["init", "--quiet"]);
        run_git(&repository, &["config", "user.name", "RustClaw Test"]);
        run_git(
            &repository,
            &["config", "user.email", "rustclaw-test@example.invalid"],
        );
        Self {
            workspace,
            repository,
        }
    }

    fn commit_file(&self, path: &str, content: &str, message: &str) -> String {
        let file = self.repository.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(&file, content).expect("write fixture");
        run_git(&self.repository, &["add", "--", path]);
        run_git(&self.repository, &["commit", "--quiet", "-m", message]);
        git_stdout(&self.repository, &["rev-parse", "HEAD"])
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.workspace);
    }
}

fn run_git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .expect("run git fixture query");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output is utf-8")
        .trim()
        .to_string()
}

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.git_basic.execution_failed");
    assert_eq!(extra["retryable"], false);
}

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
    assert_eq!(
        extra
            .get("paths")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        extra
            .pointer("/field_value/paths")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.as_str()),
        Some("Cargo.toml")
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
    assert_eq!(
        changed
            .pointer("/field_value/paths")
            .and_then(|value| value.as_array())
            .and_then(|items| items.get(1))
            .and_then(|value| value.as_str()),
        Some("src/main.rs")
    );
}

#[test]
fn remote_extra_exposes_scalar_machine_fields_for_delivery() {
    let extra = git_success_extra(
        "remote",
        "remote",
        "remote",
        0,
        "origin\tgit@example.com:repo.git (fetch)\norigin\tgit@example.com:repo.git (push)\nbackup\tssh://example/backup.git (fetch)\n",
        "exit=0\norigin\tgit@example.com:repo.git (fetch)\norigin\tgit@example.com:repo.git (push)\nbackup\tssh://example/backup.git (fetch)\n",
        None,
    );

    assert_eq!(
        extra
            .pointer("/field_value/remotes")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        extra
            .pointer("/field_value/remotes")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.as_str()),
        Some("origin")
    );
    assert_eq!(
        extra
            .get("remote_urls")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
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

#[test]
fn log_success_extra_exposes_subject_for_generic_consumers() {
    let extra = git_success_extra(
        "log",
        "log",
        "log",
        0,
        "abc123 First commit\ndef456 Second subject\n",
        "exit=0\nabc123 First commit\ndef456 Second subject\n",
        None,
    );

    assert_eq!(
        extra.get("subject").and_then(|value| value.as_str()),
        Some("First commit")
    );
    assert_eq!(
        extra
            .pointer("/field_value/subject")
            .and_then(|value| value.as_str()),
        Some("First commit")
    );
    assert_eq!(
        extra
            .get("subjects")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn execute_rejects_repository_and_file_paths_outside_workspace() {
    let fixture = TestRepository::new();

    let repository_error = execute_with_workspace_root(
        &fixture.workspace,
        json!({"action": "status", "repo": "../outside"}),
    )
    .expect_err("repository traversal must fail");
    assert!(matches!(
        repository_error.code,
        "git_path_outside_workspace" | "git_repository_outside_workspace"
    ));

    fixture.commit_file("README.md", "first\n", "first");
    let file_error = execute_with_workspace_root(
        &fixture.workspace,
        json!({
            "action": "show_file_at_rev",
            "repo": "repository",
            "target": "HEAD",
            "path": "../outside"
        }),
    )
    .expect_err("file traversal must fail");
    assert_eq!(file_error.code, "git_path_outside_workspace");
}

#[test]
fn execute_rejects_option_like_revision_tokens() {
    let fixture = TestRepository::new();
    fixture.commit_file("README.md", "first\n", "first");

    let error = execute_with_workspace_root(
        &fixture.workspace,
        json!({
            "action": "show",
            "repo": "repository",
            "target": "--help"
        }),
    )
    .expect_err("option-like revision must fail");

    assert_eq!(error.code, "git_revision_invalid");
}

#[test]
fn rev_parse_honors_requested_ref_and_returns_exact_revision() {
    let fixture = TestRepository::new();
    let first = fixture.commit_file("README.md", "first\n", "first");
    fixture.commit_file("README.md", "second\n", "second");

    let (_, extra) = execute_with_workspace_root(
        &fixture.workspace,
        json!({
            "action": "rev_parse",
            "repo": "repository",
            "ref": "HEAD~1"
        }),
    )
    .expect("resolve prior revision");

    assert_eq!(
        extra.get("revision").and_then(|value| value.as_str()),
        Some(first.as_str())
    );
    assert_eq!(
        extra
            .pointer("/provenance/source")
            .and_then(|value| value.as_str()),
        Some("git_cli")
    );
}

#[test]
fn show_file_at_rev_uses_resolved_revision_and_preserves_full_output() {
    let fixture = TestRepository::new();
    let content = "0123456789abcdef\n".repeat(1024);
    let revision = fixture.commit_file("large.txt", &content, "large file");

    let (text, extra) = execute_with_workspace_root(
        &fixture.workspace,
        json!({
            "action": "show_file_at_rev",
            "repo": "repository",
            "target": "HEAD",
            "path": "large.txt"
        }),
    )
    .expect("read file at revision");

    assert!(text.contains(&content));
    assert!(!text.contains("...(truncated)"));
    assert_eq!(
        extra.get("revision").and_then(|value| value.as_str()),
        Some(revision.as_str())
    );
    assert_eq!(
        extra.get("content_bytes").and_then(|value| value.as_u64()),
        Some(content.len() as u64)
    );
    assert!(extra
        .get("output_sha256")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.starts_with("sha256:")));
    assert_eq!(
        extra.get("truncated").and_then(|value| value.as_bool()),
        Some(false)
    );
}

#[test]
fn log_cursor_pages_are_stable_and_non_overlapping() {
    let fixture = TestRepository::new();
    fixture.commit_file("counter.txt", "one\n", "commit one");
    fixture.commit_file("counter.txt", "two\n", "commit two");
    fixture.commit_file("counter.txt", "three\n", "commit three");

    let (_, first_page) = execute_with_workspace_root(
        &fixture.workspace,
        json!({
            "action": "log",
            "repo": "repository",
            "cursor": 0,
            "limit": 1
        }),
    )
    .expect("first log page");
    let next_cursor = first_page
        .pointer("/page/next_cursor")
        .and_then(|value| value.as_u64())
        .expect("next cursor");
    let first_sha = first_page
        .pointer("/commits/0/sha")
        .and_then(|value| value.as_str())
        .expect("first sha")
        .to_string();

    let (_, second_page) = execute_with_workspace_root(
        &fixture.workspace,
        json!({
            "action": "log",
            "repo": "repository",
            "cursor": next_cursor,
            "limit": 1
        }),
    )
    .expect("second log page");
    let second_sha = second_page
        .pointer("/commits/0/sha")
        .and_then(|value| value.as_str())
        .expect("second sha");

    assert_ne!(first_sha, second_sha);
    assert_eq!(
        first_page
            .pointer("/page/returned_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        first_page
            .pointer("/page/has_more")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        second_page
            .pointer("/page/previous_cursor")
            .and_then(|value| value.as_u64()),
        Some(0)
    );
}
