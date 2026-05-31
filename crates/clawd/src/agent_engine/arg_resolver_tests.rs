use super::{normalize_skill_arg_aliases, resolve_arg_string, rewrite_args_with_auto_locator_path};
use crate::{agent_engine::LoopState, IntentOutputContract, OutputLocatorKind};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_arg_resolver_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn resolve_arg_string_replaces_trimmed_double_brace_placeholders() {
    let mut loop_state = LoopState::new(1);
    loop_state
        .output_vars
        .insert("last_output[1]".to_string(), "clawd.log".to_string());
    loop_state
        .output_vars
        .insert("last_output.0".to_string(), "act_plan.log".to_string());

    assert_eq!(
        resolve_arg_string("/logs/{{ last_output[1] }}", &loop_state),
        "/logs/clawd.log"
    );
    assert_eq!(
        resolve_arg_string("/logs/{{last_output.0}}", &loop_state),
        "/logs/act_plan.log"
    );
}

#[test]
fn auto_locator_rewrites_system_basic_file_path() {
    let root = TempDirGuard::new("readme_file");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# title\n").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), readme_path.clone());
    let mut args = json!({
        "action": "read_range",
        "path": "/tmp/README",
        "mode": "head",
        "n": 20
    });
    assert!(rewrite_args_with_auto_locator_path(
        "system_basic",
        &mut args,
        &loop_state
    ));
    assert_eq!(
        args.get("path").and_then(|v| v.as_str()),
        Some(readme_path.as_str())
    );
}

#[test]
fn auto_locator_rewrites_directory_root_for_find_path() {
    let root = TempDirGuard::new("workspace_dir");
    let document = root.path.join("document");
    fs::create_dir_all(&document).expect("create document");
    let document_path = document.display().to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), document_path.clone());
    // 用一个明确不存在的 root，以贴近 AUTO_LOCATOR 的"兜底猜测路径"语义。
    let mut args = json!({
        "action": "find_path",
        "root": "/nonexistent_root_for_auto_locator_test_xyz",
        "name": "manual_note.txt"
    });
    assert!(rewrite_args_with_auto_locator_path(
        "system_basic",
        &mut args,
        &loop_state
    ));
    assert_eq!(
        args.get("root").and_then(|v| v.as_str()),
        Some(document_path.as_str())
    );
}

#[test]
fn fs_search_aliases_normalize_to_supported_contract() {
    let mut args = json!({
        "name_pattern": "*abcd*",
        "search_root": "/tmp/stem_unique",
        "limit": 25,
        "match_mode": "substring"
    });

    assert!(normalize_skill_arg_aliases("fs_search", &mut args));
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("find_name")
    );
    assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("abcd"));
    assert_eq!(args.get("max_results").and_then(|v| v.as_u64()), Some(25));
    assert_eq!(
        args.get("root").and_then(|v| v.as_str()),
        Some("/tmp/stem_unique")
    );
}

#[test]
fn fs_search_find_path_contract_normalizes_to_find_name() {
    let mut args = json!({
        "action": "find_path",
        "path": "/tmp/workspace",
        "target": "archive",
        "limit": 10
    });

    assert!(normalize_skill_arg_aliases("fs_search", &mut args));
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("find_name")
    );
    assert_eq!(
        args.get("pattern").and_then(|v| v.as_str()),
        Some("archive")
    );
    assert_eq!(
        args.get("root").and_then(|v| v.as_str()),
        Some("/tmp/workspace")
    );
    assert!(args.get("path").is_none());
    assert!(args.get("target").is_none());
}

#[test]
fn fs_search_max_entries_alias_normalizes_to_max_results() {
    let mut args = json!({
        "action": "find_name",
        "pattern": "abcd",
        "search_root": "/tmp/workspace",
        "max_entries": 4
    });

    assert!(normalize_skill_arg_aliases("fs_search", &mut args));
    assert_eq!(args.get("max_results").and_then(|v| v.as_u64()), Some(4));
}

#[test]
fn fs_search_globish_find_name_pattern_normalizes_to_contains_pattern() {
    let mut args = json!({
        "action": "find_name",
        "pattern": "*report.md*",
        "directory": "/tmp/docs"
    });

    assert!(normalize_skill_arg_aliases("fs_search", &mut args));
    assert_eq!(
        args.get("pattern").and_then(|v| v.as_str()),
        Some("report.md")
    );
    assert_eq!(args.get("root").and_then(|v| v.as_str()), Some("/tmp/docs"));
}

#[test]
fn fs_search_find_content_alias_normalizes_to_name_search_contract() {
    let mut args = json!({
        "action": "find_content",
        "query": "abcd",
        "dir": "/tmp/stem_unique"
    });

    assert!(normalize_skill_arg_aliases("fs_search", &mut args));
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("find_name")
    );
    assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("abcd"));
    assert_eq!(
        args.get("root").and_then(|v| v.as_str()),
        Some("/tmp/stem_unique")
    );
}

#[test]
fn fs_search_find_ext_aliases_preserve_multi_extension_contract() {
    let mut args = json!({
        "action": "search_extension",
        "extensions": ["md", "txt"],
        "query": "log",
        "directory": "/tmp/docs"
    });

    assert!(normalize_skill_arg_aliases("fs_search", &mut args));
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("find_ext")
    );
    assert_eq!(
        args.get("ext").and_then(|v| v.as_array()).map(Vec::len),
        Some(2)
    );
    assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("log"));
    assert_eq!(args.get("root").and_then(|v| v.as_str()), Some("/tmp/docs"));
}

#[test]
fn auto_locator_sets_missing_fs_search_root() {
    let root = TempDirGuard::new("fs_search_auto_root");
    let search_root = root.path.join("stem_unique");
    fs::create_dir_all(&search_root).expect("create search root");
    let search_root_path = search_root.display().to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), search_root_path.clone());
    loop_state.output_contract = Some(IntentOutputContract {
        exact_sentence_count: None,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: search_root_path.clone(),
        ..IntentOutputContract::default()
    });
    let mut args = json!({
        "action": "find_name",
        "pattern": "abcd"
    });

    assert!(rewrite_args_with_auto_locator_path(
        "fs_search",
        &mut args,
        &loop_state
    ));
    assert_eq!(
        args.get("root").and_then(|v| v.as_str()),
        Some(search_root_path.as_str())
    );
}

#[test]
fn auto_locator_overwrites_missing_fs_search_root() {
    let root = TempDirGuard::new("fs_search_missing_root");
    let search_root = root.path.join("case_only");
    fs::create_dir_all(&search_root).expect("create search root");
    let search_root_path = search_root.display().to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), search_root_path.clone());
    loop_state.output_contract = Some(IntentOutputContract {
        exact_sentence_count: None,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: search_root_path.clone(),
        ..IntentOutputContract::default()
    });
    let mut args = json!({
        "action": "find_name",
        "pattern": "report.md",
        "root": "/nonexistent_case_only"
    });

    assert!(rewrite_args_with_auto_locator_path(
        "fs_search",
        &mut args,
        &loop_state
    ));
    assert_eq!(
        args.get("root").and_then(|v| v.as_str()),
        Some(search_root_path.as_str())
    );
}

#[test]
fn auto_locator_preserves_explicit_existing_path() {
    // F8 回归用例：当 LLM 显式给的 path 是真实存在的具体文件时（典型场景：
    // 多文件 read 链路第二个 read_file），AUTO_LOCATOR 不得覆盖它。
    let root = TempDirGuard::new("explicit_existing");
    let readme = root.path.join("README.md");
    let notes = root.path.join("notes.md");
    fs::write(&readme, "# readme\n").expect("write readme");
    fs::write(&notes, "# notes\n").expect("write notes");
    let mut loop_state = LoopState::new(2);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), notes.display().to_string());
    let mut args = json!({"path": readme.display().to_string()});
    let rewritten = rewrite_args_with_auto_locator_path("read_file", &mut args, &loop_state);
    assert!(!rewritten, "explicit existing path must not be rewritten");
    assert_eq!(
        args.get("path").and_then(|v| v.as_str()),
        Some(readme.display().to_string().as_str())
    );
}

#[test]
fn broad_current_workspace_auto_locator_does_not_overwrite_missing_inventory_path() {
    let root = TempDirGuard::new("broad_current_workspace");
    let root_path = root.path.display().to_string();
    let explicit_missing = root.path.join("archive").display().to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), root_path);
    loop_state.output_contract = Some(IntentOutputContract {
        exact_sentence_count: None,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    });
    let mut args = json!({
        "action": "inventory_dir",
        "path": explicit_missing,
        "depth": 1
    });

    let rewritten = rewrite_args_with_auto_locator_path("system_basic", &mut args, &loop_state);

    assert!(
        !rewritten,
        "broad workspace fallback must not silently replace a concrete missing path"
    );
    assert_eq!(
        args.get("path").and_then(|v| v.as_str()),
        Some(explicit_missing.as_str())
    );
}

#[test]
fn concrete_auto_locator_still_overwrites_missing_inventory_path() {
    let root = TempDirGuard::new("concrete_locator");
    let archive = root.path.join("docs_archive");
    fs::create_dir_all(&archive).expect("create archive");
    let archive_path = archive.display().to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), archive_path.clone());
    loop_state.output_contract = Some(IntentOutputContract {
        exact_sentence_count: None,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: archive_path.clone(),
        ..IntentOutputContract::default()
    });
    let mut args = json!({
        "action": "inventory_dir",
        "path": "/nonexistent_dir_for_concrete_auto_locator_test_xyz",
        "depth": 1
    });

    let rewritten = rewrite_args_with_auto_locator_path("system_basic", &mut args, &loop_state);

    assert!(
        rewritten,
        "concrete locator should repair guessed missing paths"
    );
    assert_eq!(
        args.get("path").and_then(|v| v.as_str()),
        Some(archive_path.as_str())
    );
}
