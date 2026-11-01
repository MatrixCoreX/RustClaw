use super::*;
use crate::runtime::host_filesystem::HostFilesystemPolicy;
use std::os::unix::fs::DirBuilderExt;

fn private_repository(root: &std::path::Path) -> PathBuf {
    let private = root.join("z-private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(root.join("a_public.rs"), "pub fn public_marker() {}\n").unwrap();
    fs::write(private.join("marker.rs"), "pub fn private_marker() {}\n").unwrap();
    private
}

fn fixture_git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .env_clear()
        .envs(crate::skills::SkillRunnerEnvironment::capture().into_pairs())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
}

async fn repository_filter_is_not_executed(driver_kind: &str, info_attributes: bool) {
    let root = TempDirGuard::new("host_private_index_git_filter");
    let private = private_repository(&root.path);
    fs::write(
        root.path.join(".gitattributes"),
        "*.rs filter=owned_fixture\n",
    )
    .unwrap();
    fixture_git(&root.path, &["init", "--quiet"]);
    fixture_git(&root.path, &["add", "a_public.rs", ".gitattributes"]);
    fixture_git(
        &root.path,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    if info_attributes {
        fs::write(
            root.path.join(".git/info/attributes"),
            "*.rs filter=owned_fixture\n",
        )
        .unwrap();
        fs::remove_file(root.path.join(".gitattributes")).unwrap();
    }
    fixture_git(
        &root.path,
        &["config", "filter.owned_fixture.required", "true"],
    );
    fixture_git(
        &root.path,
        &[
            "config",
            &format!("filter.owned_fixture.{driver_kind}"),
            "exit 73",
        ],
    );
    fs::write(
        root.path.join("a_public.rs"),
        "pub fn public_changed() {}\n",
    )
    .unwrap();
    let index_before = fs::read(root.path.join(".git/index")).unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    let result = execute_builtin_skill(&state, "code_index", &json!({"action":"changed_impact"}))
        .await
        .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    let failures = result["data"]["git_query_failures"].as_u64().unwrap();
    eprintln!("INDEX_GIT_FILTER actual_builtin=true driver={driver_kind} query_failures={failures} llm_calls=0");
    assert_eq!(
        fs::read(root.path.join(".git/index")).unwrap(),
        index_before
    );
    assert_eq!(
        failures, 0,
        "repository filter commands must not execute during observation"
    );
    assert!(result["data"]["changed_paths"]
        .as_array()
        .unwrap()
        .contains(&json!("a_public.rs")));
    assert!(!root.path.join(".agent-runtime").exists());
}

#[tokio::test]
async fn host_private_index_git_queries_do_not_execute_repository_clean_filters() {
    repository_filter_is_not_executed("clean", false).await;
}

#[tokio::test]
async fn host_private_index_git_queries_do_not_execute_repository_process_filters() {
    repository_filter_is_not_executed("process", false).await;
}

#[tokio::test]
async fn host_private_index_git_queries_do_not_execute_info_attribute_clean_filters() {
    repository_filter_is_not_executed("clean", true).await;
}

#[tokio::test]
async fn host_private_index_git_queries_do_not_execute_info_attribute_process_filters() {
    repository_filter_is_not_executed("process", true).await;
}

fn initialize_fixture_repository(root: &std::path::Path) {
    fixture_git(root, &["init", "--quiet"]);
    fixture_git(root, &["add", "."]);
    fixture_git(
        root,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
}

async fn query_changed_paths(state: &AppState) -> Value {
    let result = execute_builtin_skill(state, "code_index", &json!({"action":"changed_impact"}))
        .await
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[tokio::test]
async fn host_private_index_git_shadow_preserves_subdirectory_scope_and_ignored_files() {
    let root = TempDirGuard::new("host_private_index_git_subdirectory");
    fs::create_dir(root.path.join("sub")).unwrap();
    fs::write(root.path.join("sub/source.rs"), "pub fn original() {}\n").unwrap();
    fs::write(root.path.join("outside.rs"), "pub fn outside() {}\n").unwrap();
    initialize_fixture_repository(&root.path);
    fs::write(root.path.join(".git/info/exclude"), "sub/ignored.rs\n").unwrap();
    fs::write(root.path.join("sub/source.rs"), "pub fn changed() {}\n").unwrap();
    fs::write(
        root.path.join("outside.rs"),
        "pub fn outside_changed() {}\n",
    )
    .unwrap();
    fs::write(root.path.join("sub/ignored.rs"), "pub fn ignored() {}\n").unwrap();
    for name in [
        " leading.rs",
        "trailing.rs ",
        "line\nname.rs",
        "literal\\name.rs",
    ] {
        fs::write(root.path.join("sub").join(name), "pub fn untracked() {}\n").unwrap();
    }
    let state = test_state(root.path.join("sub"));
    let result = query_changed_paths(&state).await;
    assert_eq!(result["data"]["git_query_failures"], 0);
    assert_eq!(result["data"]["changed_paths_complete"], true);
    let paths = result["data"]["changed_paths"].as_array().unwrap();
    for name in [
        "source.rs",
        " leading.rs",
        "trailing.rs ",
        "line\nname.rs",
        "literal\\name.rs",
    ] {
        assert!(paths.contains(&json!(name)), "{result}");
    }
    assert_eq!(
        paths.len(),
        5,
        "must not include ignored files or sibling paths"
    );
}

#[tokio::test]
async fn host_private_index_git_shadow_preserves_linked_worktree_head_and_metadata() {
    let origin = TempDirGuard::new("host_private_index_git_origin");
    fs::write(origin.path.join("source.rs"), "pub fn original() {}\n").unwrap();
    initialize_fixture_repository(&origin.path);
    let linked = TempDirGuard::new("host_private_index_git_linked");
    fixture_git(
        &origin.path,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.path.to_str().unwrap(),
            "HEAD",
        ],
    );
    fs::write(
        linked.path.join("source.rs"),
        "pub fn worktree_changed() {}\n",
    )
    .unwrap();
    let gitfile = fs::read(linked.path.join(".git")).unwrap();
    let state = test_state(linked.path.clone());
    let result = query_changed_paths(&state).await;
    assert_eq!(result["data"]["git_query_failures"], 0, "{result}");
    assert!(result["data"]["changed_paths"]
        .as_array()
        .unwrap()
        .contains(&json!("source.rs")));
    assert_eq!(fs::read(linked.path.join(".git")).unwrap(), gitfile);
}

#[tokio::test]
async fn host_private_index_git_shadow_preserves_split_index_without_writing_originals() {
    let root = TempDirGuard::new("host_private_index_git_split");
    fs::write(root.path.join("source.rs"), "pub fn original() {}\n").unwrap();
    initialize_fixture_repository(&root.path);
    fixture_git(&root.path, &["update-index", "--split-index"]);
    let before = fs::read(root.path.join(".git/index")).unwrap();
    fs::write(root.path.join("source.rs"), "pub fn split_changed() {}\n").unwrap();
    let state = test_state(root.path.clone());
    let result = query_changed_paths(&state).await;
    assert_eq!(result["data"]["git_query_failures"], 0, "{result}");
    assert!(result["data"]["changed_paths"]
        .as_array()
        .unwrap()
        .contains(&json!("source.rs")));
    assert_eq!(fs::read(root.path.join(".git/index")).unwrap(), before);
}

#[tokio::test]
async fn host_private_index_git_shadow_does_not_run_nested_repository_status_filters() {
    let root = TempDirGuard::new("host_private_index_git_submodule");
    let nested = root.path.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("source.rs"), "pub fn original() {}\n").unwrap();
    fs::write(nested.join(".gitattributes"), "*.rs filter=owned_fixture\n").unwrap();
    initialize_fixture_repository(&nested);
    fs::write(root.path.join("source.rs"), "pub fn outer() {}\n").unwrap();
    initialize_fixture_repository(&root.path);
    fixture_git(
        &nested,
        &["config", "filter.owned_fixture.clean", "exit 73"],
    );
    fixture_git(
        &nested,
        &["config", "filter.owned_fixture.required", "true"],
    );
    fs::write(nested.join("source.rs"), "pub fn nested_changed() {}\n").unwrap();
    let state = test_state(root.path.clone());
    let result = query_changed_paths(&state).await;
    assert_eq!(result["data"]["git_query_failures"], 0, "{result}");
    assert_eq!(result["data"]["git_submodule_worktrees_uninspected"], 0);
    assert_eq!(result["data"]["changed_paths_complete"], true);
    assert!(result["data"]["changed_paths"]
        .as_array()
        .unwrap()
        .contains(&json!("nested/source.rs")));
}

#[tokio::test]
async fn host_private_index_git_shadow_marks_uninitialized_submodules_incomplete() {
    let root = TempDirGuard::new("host_private_index_git_uninitialized");
    let nested = root.path.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("source.rs"), "pub fn original() {}\n").unwrap();
    initialize_fixture_repository(&nested);
    fs::write(root.path.join("source.rs"), "pub fn outer() {}\n").unwrap();
    initialize_fixture_repository(&root.path);
    fs::remove_dir_all(&nested).unwrap();
    fs::create_dir(&nested).unwrap();
    let result = query_changed_paths(&test_state(root.path.clone())).await;
    assert_eq!(result["data"]["git_query_failures"], 0, "{result}");
    assert_eq!(result["data"]["git_submodule_worktrees_uninspected"], 1);
    assert_eq!(result["data"]["changed_paths_complete"], false);
}

#[tokio::test]
async fn host_private_index_git_shadow_detects_repeated_submodule_metadata_without_recursing_forever(
) {
    let root = TempDirGuard::new("host_private_index_git_cycle");
    let nested = root.path.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("source.rs"), "pub fn original() {}\n").unwrap();
    initialize_fixture_repository(&nested);
    fs::write(root.path.join("source.rs"), "pub fn outer() {}\n").unwrap();
    initialize_fixture_repository(&root.path);
    fs::remove_dir_all(nested.join(".git")).unwrap();
    fs::write(
        nested.join(".git"),
        format!("gitdir: {}\n", root.path.join(".git").display()),
    )
    .unwrap();
    let result = query_changed_paths(&test_state(root.path.clone())).await;
    assert_eq!(
        result["data"]["git_submodule_worktrees_uninspected"], 1,
        "{result}"
    );
    assert_eq!(result["data"]["changed_paths_complete"], false);
}

#[tokio::test]
async fn host_private_index_git_shadow_retains_builtin_line_ending_normalization() {
    let root = TempDirGuard::new("host_private_index_git_eol");
    fs::write(root.path.join("source.rs"), "pub fn original() {}\n").unwrap();
    initialize_fixture_repository(&root.path);
    fixture_git(&root.path, &["config", "core.autocrlf", "input"]);
    fs::write(root.path.join("source.rs"), "pub fn original() {}\r\n").unwrap();
    let result = query_changed_paths(&test_state(root.path.clone())).await;
    assert_eq!(result["data"]["git_query_failures"], 0, "{result}");
    assert_eq!(result["data"]["changed_paths"], json!([]));
    assert_eq!(result["data"]["changed_paths_complete"], true);
}

#[tokio::test]
async fn host_private_index_git_shadow_supports_sha256_repository_objects() {
    let root = TempDirGuard::new("host_private_index_git_sha256");
    fs::write(root.path.join("source.rs"), "pub fn original() {}\n").unwrap();
    fixture_git(&root.path, &["init", "--quiet", "--object-format=sha256"]);
    fixture_git(&root.path, &["add", "source.rs"]);
    fixture_git(
        &root.path,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    fs::write(root.path.join("source.rs"), "pub fn changed_sha256() {}\n").unwrap();
    let result = query_changed_paths(&test_state(root.path.clone())).await;
    assert_eq!(result["data"]["git_query_failures"], 0, "{result}");
    assert!(result["data"]["changed_paths"]
        .as_array()
        .unwrap()
        .contains(&json!("source.rs")));
}

#[tokio::test]
async fn host_private_index_refuses_explicit_context_and_handles_moved_private_roots() {
    let root = TempDirGuard::new("host_private_index_moved");
    let private = private_repository(&root.path);
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private.clone()], &root.path).unwrap());
    execute_builtin_skill(&state, "code_index", &json!({"action":"refresh"}))
        .await
        .unwrap();
    let cache: Value = serde_json::from_slice(
        &fs::read(root.path.join(".agent-runtime/index/repository-v1.json")).unwrap(),
    )
    .unwrap();
    assert!(cache["files"].get("a_public.rs").is_some());
    assert!(cache["files"].get("z-private/marker.rs").is_none());
    fs::rename(&private, root.path.join("moved-private")).unwrap();
    let result = execute_builtin_skill(
        &state,
        "code_index",
        &json!({"action":"find_definitions", "symbol":"private_marker"}),
    )
    .await
    .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    assert!(result["data"]["definitions"].as_array().unwrap().is_empty());
    assert!(
        result["summary"]["policy_excluded_entries"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(execute_builtin_skill(
        &state,
        "code_index",
        &json!({"action":"retrieve_context", "paths":["moved-private/marker.rs"]})
    )
    .await
    .is_err());
}

#[tokio::test]
async fn host_private_index_refuses_a_protected_cache_without_creating_or_overwriting_files() {
    let root = TempDirGuard::new("host_private_index_cache");
    let private = root.path.join(".agent-runtime/index");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&private)
        .unwrap();
    fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    fs::write(root.path.join("public.rs"), "pub fn public_marker() {}\n").unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private.clone()], &root.path).unwrap());
    assert!(
        execute_builtin_skill(&state, "code_index", &json!({"action":"refresh"}))
            .await
            .is_err()
    );
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"owned-private-marker"
    );
    assert_eq!(fs::read_dir(private).unwrap().count(), 1);
}

#[tokio::test]
async fn host_private_index_git_discovery_keeps_public_changes_and_filters_private_cached_names() {
    let root = TempDirGuard::new("host_private_index_git");
    let private = private_repository(&root.path);
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "a_public.rs", "z-private/marker.rs"],
        vec![
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .env_clear()
            .envs(crate::skills::SkillRunnerEnvironment::capture().into_pairs())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(&root.path)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    fs::write(
        root.path.join("a_public.rs"),
        "pub fn public_changed() {}\n",
    )
    .unwrap();
    fs::write(private.join("marker.rs"), "pub fn private_changed() {}\n").unwrap();
    fs::write(
        root.path.join("public\nname.rs"),
        "pub fn newline_marker() {}\n",
    )
    .unwrap();
    let git_index_before = fs::read(root.path.join(".git/index")).unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    let result = execute_builtin_skill(&state, "code_index", &json!({"action":"changed_impact"}))
        .await
        .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    let paths = result["data"]["changed_paths"].as_array().unwrap();
    assert!(paths.contains(&json!("a_public.rs")));
    assert!(paths.contains(&json!("public\nname.rs")));
    assert!(!paths.contains(&json!("z-private/marker.rs")));
    assert_eq!(result["data"]["changed_paths_complete"], false);
    assert_eq!(result["data"]["git_query_failures"], 0);
    assert!(result["data"]["policy_excluded_paths"].as_u64().unwrap() > 0);
    assert_eq!(
        fs::read(root.path.join(".git/index")).unwrap(),
        git_index_before
    );
}

#[tokio::test]
async fn host_private_index_excludes_private_sources_without_disabling_public_queries() {
    let root = TempDirGuard::new("host_private_index_fresh");
    let private = private_repository(&root.path);
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    let public = execute_builtin_skill(
        &state,
        "code_index",
        &json!({
            "action":"find_definitions", "symbol":"public_marker"
        }),
    )
    .await
    .unwrap();
    let public: Value = serde_json::from_str(&public).unwrap();
    assert_eq!(public["data"]["definitions"].as_array().unwrap().len(), 1);
    let result = execute_builtin_skill(
        &state,
        "code_index",
        &json!({
            "action":"find_definitions", "symbol":"private_marker"
        }),
    )
    .await
    .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    let count = result["data"]["definitions"].as_array().unwrap().len();
    eprintln!("HOST_PRIVATE_INDEX actual_builtin=true cached=false private_definition_count={count} llm_calls=0");
    assert_eq!(
        count, 0,
        "the index must not expose protected source definitions"
    );
    assert!(
        result["summary"]["policy_excluded_entries"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(result["summary"]["scan_complete"], false);
    assert!(!root.path.join(".agent-runtime").exists());
}

#[tokio::test]
async fn host_private_index_revalidates_retained_cache_before_a_partial_query() {
    let root = TempDirGuard::new("host_private_index_cached");
    let private = private_repository(&root.path);
    let mut state = test_state(root.path.clone());
    execute_builtin_skill(&state, "code_index", &json!({"action":"refresh"}))
        .await
        .unwrap();
    let cache_path = root.path.join(".agent-runtime/index/repository-v1.json");
    let cache_before = fs::read(&cache_path).unwrap();
    assert!(
        serde_json::from_slice::<Value>(&cache_before).unwrap()["files"]
            .get("z-private/marker.rs")
            .is_some()
    );
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    let result = execute_builtin_skill(
        &state,
        "code_index",
        &json!({
            "action":"find_definitions", "symbol":"private_marker", "max_files":1
        }),
    )
    .await
    .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    let count = result["data"]["definitions"].as_array().unwrap().len();
    eprintln!("HOST_PRIVATE_INDEX actual_builtin=true cached=true private_definition_count={count} llm_calls=0");
    assert_eq!(
        count, 0,
        "a retained index is not current filesystem authorization"
    );
    assert!(result["summary"]["cache_rejected_files"].as_u64().unwrap() > 0);
    assert!(
        result["summary"]["policy_excluded_entries"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(result["summary"]["scan_complete"], false);
    assert_eq!(
        fs::read(cache_path).unwrap(),
        cache_before,
        "queries remain observe-only"
    );
}
