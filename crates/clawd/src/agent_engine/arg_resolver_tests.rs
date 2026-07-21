use super::{
    normalize_skill_arg_aliases, resolve_arg_string, resolve_arg_value,
    rewrite_args_with_auto_locator_path,
};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
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

fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    }
}

#[test]
fn resolve_arg_string_replaces_trimmed_double_brace_placeholders() {
    let mut loop_state = LoopState::new();
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
fn resolve_arg_string_replaces_last_output_listing_entry_path_reference() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","path":"/workspace/logs","entries":[{"name":"model_io.log","path":"logs/model_io.log"},{"name":"act_plan.log","path":"logs/act_plan.log"}]}}"#,
    ));

    assert_eq!(
        resolve_arg_string("{{ last_output.entries.1.path }}", &loop_state),
        "logs/act_plan.log"
    );
}

#[test]
fn resolve_arg_string_replaces_steps_output_listing_entry_path_reference() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","path":"/workspace/logs","entries":[{"name":"model_io.log","path":"logs/model_io.log"}]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","path":"/workspace/document","entries":[{"name":"note.md","path":"document/note.md"}]}}"#,
    ));

    assert_eq!(
        resolve_arg_string("{{steps.0.outputs.entries[0].path}}", &loop_state),
        "logs/model_io.log"
    );
    assert_eq!(
        resolve_arg_string("{{steps.1.outputs.entries[0].path}}", &loop_state),
        "document/note.md"
    );
}

#[test]
fn resolve_arg_value_maps_file_placeholder_path_segment_from_latest_listing() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"logs","resolved_path":"/workspace/logs","entries":[{"name":"clawd.run.log","path":"logs/clawd.run.log"},{"name":"model_io.log","path":"logs/model_io.log"}]}"#,
    ));
    let args = json!({
        "action": "read_text_range",
        "path": "/workspace/logs/<file2>",
        "note": "keep <file1> as prose"
    });

    let resolved = resolve_arg_value(&args, &loop_state);

    assert_eq!(
        resolved.get("path").and_then(|value| value.as_str()),
        Some("logs/model_io.log")
    );
    assert_eq!(
        resolved.get("note").and_then(|value| value.as_str()),
        Some("keep <file1> as prose")
    );
}

#[test]
fn resolve_arg_value_maps_file_placeholder_array_items_from_latest_listing_names() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"inventory_dir","path":"document","resolved_path":"/workspace/document","names":["a.md","b.md"]}"#,
    ));
    let args = json!({
        "paths": ["<file1>", "<file2>"],
        "labels": ["<file1>", "<file2>"]
    });

    let resolved = resolve_arg_value(&args, &loop_state);

    assert_eq!(
        resolved.get("paths"),
        Some(&json!([
            "/workspace/document/a.md",
            "/workspace/document/b.md"
        ]))
    );
    assert_eq!(resolved.get("labels"), Some(&json!(["<file1>", "<file2>"])));
}

#[test]
fn resolve_arg_value_maps_recent_placeholder_path_segment_from_latest_listing() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","path":"/home/guagua/rustclaw/logs","resolved_path":"/home/guagua/rustclaw/logs","entries":[{"name":"model_io.log","path":"logs/model_io.log"},{"name":"act_plan.log","path":"logs/act_plan.log"}],"files_only":true}}"#,
    ));
    let args = json!({
        "action": "read_text_range",
        "path": "/home/guagua/rustclaw/logs/<recent2>"
    });

    let resolved = resolve_arg_value(&args, &loop_state);

    assert_eq!(
        resolved.get("path").and_then(|value| value.as_str()),
        Some("logs/act_plan.log")
    );
}

#[test]
fn resolve_arg_value_maps_recent_file_placeholder_to_first_listing_entry() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"list_dir","path":"/workspace/logs","entries":[{"name":"model_io.log"},{"name":"act_plan.log"}]}"#,
    ));
    let args = json!({
        "path": "<recent_file>",
        "label": "<recent_file>"
    });

    let resolved = resolve_arg_value(&args, &loop_state);

    assert_eq!(
        resolved.get("path").and_then(|value| value.as_str()),
        Some("/workspace/logs/model_io.log")
    );
    assert_eq!(
        resolved.get("label").and_then(|value| value.as_str()),
        Some("<recent_file>")
    );
}

#[test]
fn auto_locator_rewrites_system_basic_file_path() {
    let root = TempDirGuard::new("readme_file");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# title\n").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut loop_state = LoopState::new();
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
fn auto_locator_rewrites_config_basic_read_field_path() {
    let root = TempDirGuard::new("config_basic_file");
    let cargo_toml = root.path.join("Cargo.toml");
    fs::write(&cargo_toml, "[package]\nversion = \"0.1.0\"\n").expect("write config");
    let target_path = cargo_toml.display().to_string();
    let mut loop_state = LoopState::new();
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), target_path.clone());
    let mut args = json!({
        "action": "read_field",
        "path": "/tmp/wrong-Cargo.toml",
        "field_path": "package.version"
    });
    assert!(rewrite_args_with_auto_locator_path(
        "config_basic",
        &mut args,
        &loop_state
    ));
    assert_eq!(
        args.get("path").and_then(|v| v.as_str()),
        Some(target_path.as_str())
    );
}

#[test]
fn auto_locator_rewrites_directory_root_for_find_path() {
    let root = TempDirGuard::new("workspace_dir");
    let document = root.path.join("document");
    fs::create_dir_all(&document).expect("create document");
    let document_path = document.display().to_string();
    let mut loop_state = LoopState::new();
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
fn fs_basic_make_dir_create_parents_alias_normalizes_to_parents() {
    let mut args = json!({
        "action": "make_dir",
        "path": "/tmp/project",
        "create_parents": true
    });

    assert!(normalize_skill_arg_aliases("fs_basic", &mut args));
    assert_eq!(args.get("parents").and_then(|v| v.as_bool()), Some(true));
    assert!(args.get("create_parents").is_none());
}

#[test]
fn standalone_make_dir_create_parents_alias_drops_unsupported_key() {
    let mut args = json!({
        "path": "/tmp/project",
        "create_parents": true
    });

    assert!(normalize_skill_arg_aliases("make_dir", &mut args));
    assert_eq!(args, json!({"path": "/tmp/project", "parents": true}));
}

#[test]
fn run_cmd_shell_command_alias_normalizes_to_command() {
    let mut args = json!({
        "shell_command": "python3 test_calc_core.py",
        "cwd": "/tmp/project"
    });

    assert!(normalize_skill_arg_aliases("run_cmd", &mut args));
    assert_eq!(
        args.get("command").and_then(|v| v.as_str()),
        Some("python3 test_calc_core.py")
    );
    assert_eq!(
        args.get("shell_command").and_then(|v| v.as_str()),
        Some("python3 test_calc_core.py")
    );
}

#[test]
fn browser_web_open_extract_numeric_ranges_normalize_to_skill_contract() {
    let mut args = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "max_pages": 0,
        "max_text_chars": 0,
        "min_content_chars": 0,
        "content_mode": "title",
        "wait_until": "network_idle"
    });

    assert!(normalize_skill_arg_aliases("browser_web", &mut args));

    assert_eq!(args.get("max_pages").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(
        args.get("max_text_chars").and_then(|v| v.as_i64()),
        Some(100)
    );
    assert_eq!(
        args.get("min_content_chars").and_then(|v| v.as_i64()),
        Some(20)
    );
    assert_eq!(
        args.get("content_mode").and_then(|v| v.as_str()),
        Some("clean")
    );
    assert_eq!(
        args.get("wait_until").and_then(|v| v.as_str()),
        Some("networkidle")
    );
}

#[test]
fn browser_web_search_aliases_normalize_to_skill_contract() {
    let mut args = json!({
        "action": "search_extract",
        "query": "rustclaw",
        "max_results": 0,
        "max_pages": 99,
        "min_content_chars": "0"
    });

    assert!(normalize_skill_arg_aliases("browser_web", &mut args));

    assert_eq!(args.get("top_k").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(args.get("extract_top_n").and_then(|v| v.as_i64()), Some(10));
    assert_eq!(
        args.get("min_content_chars").and_then(|v| v.as_i64()),
        Some(20)
    );
}

#[test]
fn browser_web_open_extract_raw_mode_alias_normalizes_to_skill_contract() {
    let mut args = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "content_mode": "html",
        "wait_until": "page-load"
    });

    assert!(normalize_skill_arg_aliases("browser_web", &mut args));

    assert_eq!(
        args.get("content_mode").and_then(|v| v.as_str()),
        Some("raw")
    );
    assert_eq!(
        args.get("wait_until").and_then(|v| v.as_str()),
        Some("load")
    );
}

#[test]
fn image_edit_prompt_alias_normalizes_to_instruction() {
    let mut args = json!({
        "image": "https://example.test/rust.png",
        "prompt": "pixel art style",
        "output_path": "document/rust_icon_pixel_smoke.png"
    });

    assert!(normalize_skill_arg_aliases("image_edit", &mut args));
    assert_eq!(
        args.get("instruction").and_then(|value| value.as_str()),
        Some("pixel art style")
    );
}

#[test]
fn image_generate_subject_and_dimensions_aliases_normalize() {
    let mut args = json!({
        "subject": "RustClaw status card",
        "width": 512,
        "height": "512",
        "output_path": "document/media_dry_run/image_status_card.png",
        "dry_run": true
    });

    assert!(normalize_skill_arg_aliases("image_generate", &mut args));
    assert_eq!(
        args.get("prompt").and_then(|value| value.as_str()),
        Some("RustClaw status card")
    );
    assert_eq!(
        args.get("size").and_then(|value| value.as_str()),
        Some("512x512")
    );
}

#[test]
fn audio_synthesize_input_alias_normalizes_to_text() {
    let mut args = json!({
        "input": "RustClaw dry run audio check",
        "output_path": "document/media_dry_run/audio_check.mp3",
        "dry_run": true
    });

    assert!(normalize_skill_arg_aliases("audio_synthesize", &mut args));
    assert_eq!(
        args.get("text").and_then(|value| value.as_str()),
        Some("RustClaw dry run audio check")
    );
}

#[test]
fn service_control_machine_aliases_normalize_to_contract_fields() {
    let mut args = json!({
        "action": "status",
        "unit": "ssh",
        "manager": "systemd"
    });

    assert!(normalize_skill_arg_aliases("service_control", &mut args));
    assert_eq!(
        args.get("target").and_then(|value| value.as_str()),
        Some("ssh")
    );
    assert_eq!(
        args.get("manager_type").and_then(|value| value.as_str()),
        Some("systemd")
    );
}

#[test]
fn service_control_name_alias_normalizes_to_target() {
    let mut args = json!({
        "action": "status",
        "name": "sshd"
    });

    assert!(normalize_skill_arg_aliases("service_control", &mut args));
    assert_eq!(
        args.get("target").and_then(|value| value.as_str()),
        Some("sshd")
    );
}

#[test]
fn video_generate_description_alias_normalizes_to_prompt() {
    let mut args = json!({
        "description": "status panel product video",
        "duration": 6,
        "resolution": "768P",
        "output_path": "document/media_dry_run/status_panel.mp4",
        "dry_run": true
    });

    assert!(normalize_skill_arg_aliases("video_generate", &mut args));
    assert_eq!(
        args.get("prompt").and_then(|value| value.as_str()),
        Some("status panel product video")
    );
}

#[test]
fn music_generate_theme_alias_normalizes_to_prompt() {
    let mut args = json!({
        "theme": "short instrumental ambient loop",
        "format": "mp3",
        "output_path": "document/media_dry_run/ambient_loop.mp3",
        "dry_run": true
    });

    assert!(normalize_skill_arg_aliases("music_generate", &mut args));
    assert_eq!(
        args.get("prompt").and_then(|value| value.as_str()),
        Some("short instrumental ambient loop")
    );
}

#[test]
fn kb_ingest_source_alias_normalizes_to_paths_array() {
    let mut args = json!({
        "action": "ingest",
        "namespace": "demo_docs_nl",
        "source": "/home/guagua/rustclaw/README.md"
    });

    assert!(normalize_skill_arg_aliases("kb", &mut args));
    assert_eq!(
        args.get("paths").and_then(|value| value.as_array()),
        Some(&vec![json!("/home/guagua/rustclaw/README.md")])
    );
}

#[test]
fn kb_ingest_source_paths_alias_normalizes_to_paths_array() {
    let mut args = json!({
        "action": "ingest",
        "namespace": "demo_docs_nl",
        "source_paths": ["README.md", "README.zh-CN.md"]
    });

    assert!(normalize_skill_arg_aliases("kb", &mut args));
    assert_eq!(
        args.get("paths").and_then(|value| value.as_array()),
        Some(&vec![json!("README.md"), json!("README.zh-CN.md")])
    );
}

#[test]
fn kb_ingest_file_path_alias_normalizes_to_paths_array() {
    let mut args = json!({
        "action": "ingest",
        "namespace": "demo_docs_nl",
        "file_path": "/home/guagua/rustclaw/README.md"
    });

    assert!(normalize_skill_arg_aliases("kb", &mut args));
    assert_eq!(
        args.get("paths").and_then(|value| value.as_array()),
        Some(&vec![json!("/home/guagua/rustclaw/README.md")])
    );
}

#[test]
fn kb_ingest_file_path_and_kb_name_aliases_normalize() {
    let mut args = json!({
        "action": "ingest",
        "file_path": "/home/guagua/rustclaw/README.md",
        "kb_name": "demo_docs_nl"
    });

    assert!(normalize_skill_arg_aliases("kb", &mut args));
    assert_eq!(
        args.get("namespace").and_then(|value| value.as_str()),
        Some("demo_docs_nl")
    );
    assert_eq!(
        args.get("paths").and_then(|value| value.as_array()),
        Some(&vec![json!("/home/guagua/rustclaw/README.md")])
    );
}

#[test]
fn kb_search_kb_name_alias_normalizes_to_namespace() {
    let mut args = json!({
        "action": "search",
        "kb_name": "demo_docs_nl",
        "query": "deployment"
    });

    assert!(normalize_skill_arg_aliases("kb", &mut args));
    assert_eq!(
        args.get("namespace").and_then(|value| value.as_str()),
        Some("demo_docs_nl")
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
    let mut loop_state = LoopState::new();
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
    let mut loop_state = LoopState::new();
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
    let mut loop_state = LoopState::new();
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
fn auto_locator_preserves_existing_structured_field_path() {
    let root = TempDirGuard::new("explicit_existing_structured");
    let package = root.path.join("package.json");
    let cargo = root.path.join("Cargo.toml");
    fs::write(&package, r#"{"name":"react-example"}"#).expect("write package");
    fs::write(&cargo, "[package]\nname=\"clawd\"\n").expect("write cargo");
    let mut loop_state = LoopState::new();
    loop_state.output_vars.insert(
        "auto_locator_path".to_string(),
        package.display().to_string(),
    );
    let mut args = json!({
        "action": "extract_field",
        "path": cargo.display().to_string(),
        "field_path": "package.name"
    });

    let rewritten = rewrite_args_with_auto_locator_path("system_basic", &mut args, &loop_state);

    assert!(
        !rewritten,
        "explicit existing structured path must not be rewritten"
    );
    assert_eq!(
        args.get("path").and_then(|v| v.as_str()),
        Some(cargo.display().to_string().as_str())
    );
}

#[test]
fn auto_locator_preserves_conflicting_missing_file_name() {
    let root = TempDirGuard::new("auto_locator_conflicting_missing_name");
    let agents = root.path.join("AGENTS.md");
    fs::write(&agents, "# rules\n").expect("write agents");
    let mut loop_state = LoopState::new();
    loop_state.output_vars.insert(
        "auto_locator_path".to_string(),
        agents.display().to_string(),
    );
    let missing_plan = root.path.join("PLAN.md").display().to_string();
    let mut args = json!({
        "action": "read_range",
        "path": missing_plan,
        "mode": "head",
        "n": 120
    });

    let rewritten = rewrite_args_with_auto_locator_path("system_basic", &mut args, &loop_state);

    assert!(
        !rewritten,
        "conflicting concrete missing file name must not be rewritten"
    );
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(missing_plan.as_str())
    );
}

#[test]
fn broad_current_workspace_auto_locator_does_not_overwrite_missing_inventory_path() {
    let root = TempDirGuard::new("broad_current_workspace");
    let root_path = root.path.display().to_string();
    let explicit_missing = root.path.join("archive").display().to_string();
    let mut loop_state = LoopState::new();
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
    let mut loop_state = LoopState::new();
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
