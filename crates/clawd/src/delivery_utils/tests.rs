use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_core::config::{AgentConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, ToolsConfig};
use reqwest::Client;
use rusqlite::Connection;
use tokio::sync::Semaphore;

use super::directory_lookup::{
    collect_directory_candidates, list_directory_entries_for_user, resolve_directory_locator_input,
    resolve_directory_target,
};
use super::file_delivery::{
    build_batch_directory_delivery_response, find_file_in_directory_non_recursive,
    format_batch_delivery_tokens, list_current_level_files_for_delivery,
    scan_filename_matches_with_limit,
};
use super::locator::{
    directory_lookup_input_from_hint, extract_bare_filename_stem_candidates,
    extract_directory_and_file_pair, extract_explicit_file_path_candidates,
    extract_filename_candidates,
};
use super::output_contract::{sync_output_payload, take_first_sentence};
use super::{
    classify_batch_directory_delivery_input, classify_directory_lookup_input,
    intercept_response_payload_for_delivery, resolve_directory_locator_for_execution,
    resolve_file_delivery_target, BatchDirectoryDeliveryResolution, CurrentLevelDeliveryEntries,
    CurrentLevelDeliveryEntriesResult, DeliveryMessageKind, DirectoryEntriesListResult,
    DirectoryFileLookupResult, DirectoryLocatorExecutionResolution, DirectoryLookupInput,
    DirectoryLookupResolution, FileDeliveryTargetResolution, FilenameScanResult,
};
use crate::{
    runtime::{AgentRuntimeConfig, RateLimiter, SkillViewsSnapshot},
    AppState, CommandIntentRuntime, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, ScheduleRuntime, ToolsPolicy,
};

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
            "clawd_delivery_locator_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_text_file(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, b"ok").expect("write file");
}

fn contract_with_delivery_intent(
    delivery_intent: OutputDeliveryIntent,
    locator_hint: &str,
) -> IntentOutputContract {
    IntentOutputContract {
        delivery_intent,
        semantic_kind: Default::default(),
        locator_hint: locator_hint.to_string(),
        ..IntentOutputContract::default()
    }
}

fn test_state_with_i18n(translations: &[(&str, &str)]) -> AppState {
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    let i18n_dict = translations
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect::<HashMap<_, _>>();
    AppState {
        started_at: std::time::Instant::now(),
        queue_limit: 1,
        db: Arc::new(Mutex::new(Connection::open_in_memory().expect("open db"))),
        llm_providers: Vec::new(),
        agents_by_id: Arc::new(agents_by_id),
        skill_timeout_seconds: 30,
        skill_runner_path: std::path::PathBuf::new(),
        skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
            registry: None,
            skills_list: Arc::new(HashSet::new()),
        }))),
        skill_semaphore: Arc::new(Semaphore::new(1)),
        rate_limiter: Arc::new(Mutex::new(RateLimiter::new(60, 30))),
        llm_calls_per_task: Arc::new(Mutex::new(HashMap::new())),
        llm_elapsed_per_task: Arc::new(Mutex::new(HashMap::new())),
        llm_by_prompt_per_task: Arc::new(Mutex::new(HashMap::new())),
        task_schedule_intent_cache: Arc::new(Mutex::new(HashMap::new())),
        maintenance: MaintenanceConfig::default(),
        memory: MemoryConfig::default(),
        workspace_root: std::env::temp_dir(),
        default_locator_search_dir: std::env::temp_dir(),
        locator_scan_max_depth: 2,
        locator_scan_max_files: 100,
        tools_policy: Arc::new(
            ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
        ),
        active_provider_type: None,
        cmd_timeout_seconds: 30,
        max_cmd_length: 4096,
        allow_path_outside_workspace: false,
        allow_sudo: false,
        worker_task_timeout_seconds: 300,
        worker_task_heartbeat_seconds: 10,
        worker_running_no_progress_timeout_seconds: 300,
        worker_running_recovery_check_interval_seconds: 30,
        last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
        routing: RoutingConfig::default(),
        persona_prompt: String::new(),
        command_intent: CommandIntentRuntime {
            all_result_suffixes: Vec::new(),
            default_locale: "zh-CN".to_string(),
            verify_enforce_enabled: false,
        },
        schedule: ScheduleRuntime {
            timezone: "Asia/Shanghai".to_string(),
            intent_prompt_template: String::new(),
            intent_prompt_source: String::new(),
            intent_rules_template: String::new(),
            locale: "zh-CN".to_string(),
            i18n_dict,
        },
        channels: crate::ChannelConfig::default(),
        http_client: Client::new(),
        database_sqlite_path: std::path::PathBuf::new(),
        database_busy_timeout_ms: 5_000,
        self_extension: claw_core::config::SelfExtensionConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
    }
}

// Sentence-shaping behavior that still lives in the facade.
#[test]
fn take_first_sentence_keeps_dot_inside_filename() {
    let text = "The file /home/guagua/test/README.md was read successfully. Summary follows.";
    assert_eq!(
        take_first_sentence(text),
        "The file /home/guagua/test/README.md was read successfully."
    );
}

#[test]
fn take_first_sentence_keeps_dot_inside_abbreviation() {
    let text = "Use v1.2 config for rollout. Then restart service.";
    assert_eq!(take_first_sentence(text), "Use v1.2 config for rollout.");
}

#[test]
fn take_first_sentence_handles_cjk_punctuation() {
    let text = "这是第一句。这里是第二句。";
    assert_eq!(take_first_sentence(text), "这是第一句。");
}

#[test]
fn take_first_sentence_skips_markdown_heading_line() {
    let text = "# Test Workspace\nThis directory is reserved for NL regression test artifacts and wrapper scripts.\n\nNotes...";
    assert_eq!(
        take_first_sentence(text),
        "This directory is reserved for NL regression test artifacts and wrapper scripts."
    );
}

#[test]
fn take_first_sentence_skips_label_only_first_line() {
    let text = "上一句话的核心要点：\n内容：该目录用于存放自动化测试脚本。";
    assert_eq!(
        take_first_sentence(text),
        "内容：该目录用于存放自动化测试脚本。"
    );
}

#[test]
fn take_first_sentence_skips_english_label_only_first_line() {
    let text = "Summary:\nThe directory stores wrapper scripts and test artifacts.";
    assert_eq!(
        take_first_sentence(text),
        "The directory stores wrapper scripts and test artifacts."
    );
}

#[test]
fn take_first_sentence_skips_markdown_wrapped_label_line() {
    let text = "**核心重点：**\n检查下游 sample 服务稳定性，若频繁出现超时需排查网络与连接池。";
    assert_eq!(
        take_first_sentence(text),
        "检查下游 sample 服务稳定性，若频繁出现超时需排查网络与连接池。"
    );
}

#[test]
fn sync_output_payload_collapses_file_token_to_single_exit() {
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        ..IntentOutputContract::default()
    };
    let mut text = "已生成文件".to_string();
    let mut messages = vec!["说明文字".to_string(), "FILE:/tmp/report.md".to_string()];

    sync_output_payload(&contract, &mut text, &mut messages);

    assert_eq!(text, "FILE:/tmp/report.md");
    assert_eq!(messages, vec!["FILE:/tmp/report.md".to_string()]);
}

#[test]
fn sync_output_payload_wraps_existing_file_path_for_file_token_contract() {
    let tmp = TempDirGuard::new("file_token_existing_path");
    let target = tmp.path().join("report.md");
    write_text_file(&target);
    let canonical = target.canonicalize().expect("canonical target");
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        delivery_required: true,
        ..IntentOutputContract::default()
    };
    let mut text = canonical.display().to_string();
    let mut messages = vec![canonical.display().to_string()];

    sync_output_payload(&contract, &mut text, &mut messages);

    let expected = format!("FILE:{}", canonical.display());
    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected]);
}

#[test]
fn sync_output_payload_collapses_one_sentence_contract_to_single_message() {
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        ..IntentOutputContract::default()
    };
    let mut text = "一句话结论。".to_string();
    let mut messages = vec!["旧消息".to_string(), "一句话结论。".to_string()];

    sync_output_payload(&contract, &mut text, &mut messages);

    assert_eq!(text, "一句话结论。");
    assert_eq!(messages, vec!["一句话结论。".to_string()]);
}

#[test]
fn directory_purpose_summary_one_sentence_contract_preserves_multiline_listing() {
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
        ..IntentOutputContract::default()
    };
    let mut text =
        "base_skill_response_contract.md\nskill_integration_guide.md\n\n这个目录主要放说明文档、操作指引和检查清单。"
            .to_string();
    let mut messages = vec![text.clone()];

    sync_output_payload(&contract, &mut text, &mut messages);

    assert_eq!(
        text,
        "base_skill_response_contract.md\nskill_integration_guide.md\n\n这个目录主要放说明文档、操作指引和检查清单。"
    );
    assert_eq!(messages, vec![text]);
}

#[test]
fn sync_output_payload_strips_preamble_before_markdown_table() {
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut text = "Sorted descending by score:\n\n| name | score |\n| --- | --- |\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |".to_string();
    let mut messages = vec![text.clone()];

    sync_output_payload(&contract, &mut text, &mut messages);

    let expected =
        "| name | score |\n| --- | --- |\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |".to_string();
    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected]);
}

#[test]
fn intercept_response_payload_localizes_missing_file_message_to_english_request() {
    let state = test_state_with_i18n(&[(
        "clawd.msg.delivery.rule1_both_roots_miss",
        "在系统根目录和项目根目录都没有找到该文件",
    )]);
    let contract = IntentOutputContract {
        delivery_required: true,
        response_shape: OutputResponseShape::FileToken,
        ..IntentOutputContract::default()
    };

    let (text, messages) = intercept_response_payload_for_delivery(
        &state,
        "send me document/definitely_missing_runtime_case_002.txt and do not paste the content",
        true,
        &contract,
        String::new(),
        Vec::new(),
    );

    assert_eq!(text, "File not found at the provided path.");
    assert_eq!(
        messages,
        vec!["File not found at the provided path.".to_string()]
    );
}

#[test]
fn intercept_response_payload_localizes_missing_directory_message_to_english_request() {
    let mut state = test_state_with_i18n(&[(
        "clawd.msg.directory.not_found_dual_root",
        "在系统根目录和项目根目录都没有找到该目录",
    )]);
    // 关键：使用隔离的 workspace_root / default_locator_search_dir，
    // 避免与并发跑的其他测试在 /tmp 下产生的临时目录互相干扰。
    let isolated = TempDirGuard::new("missing_directory_isolated");
    state.workspace_root = isolated.path().to_path_buf();
    state.default_locator_search_dir = isolated.path().to_path_buf();
    let contract = IntentOutputContract {
        delivery_intent: OutputDeliveryIntent::DirectoryLookup,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "missing-directory".to_string(),
        ..IntentOutputContract::default()
    };

    let (text, messages) = intercept_response_payload_for_delivery(
        &state,
        "list files in missing-directory",
        false,
        &contract,
        String::new(),
        Vec::new(),
    );

    assert_eq!(text, "Directory not found at the provided path.");
    assert_eq!(
        messages,
        vec!["Directory not found at the provided path.".to_string()]
    );
}

// Single-file delivery resolution rules.
#[test]
fn rule1_explicit_file_path_hits_system_root() {
    let system_root = TempDirGuard::new("rule1_system");
    let project_root = TempDirGuard::new("rule1_project");
    let target = system_root.path().join("alpha/report.md");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "把 /alpha/report.md 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule1_explicit_file_path_hits_project_root() {
    let system_root = TempDirGuard::new("rule1_system_project_hit");
    let project_root = TempDirGuard::new("rule1_project_hit");
    let target = project_root.path().join("alpha/report.md");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "把 /alpha/report.md 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule1_explicit_file_path_case_mismatch_still_hits_project_root() {
    let system_root = TempDirGuard::new("rule1_project_case_mismatch_system");
    let project_root = TempDirGuard::new("rule1_project_case_mismatch_project");
    let target = project_root.path().join("Alpha/Report.MD");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "把 /alpha/report.md 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule1_explicit_file_path_miss_both_roots() {
    let system_root = TempDirGuard::new("rule1_system_miss");
    let project_root = TempDirGuard::new("rule1_project_miss");

    let resolved = resolve_file_delivery_target(
        "把 /not_exists/report.md 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule1BothRootsMiss
        ))
    );
}

#[test]
fn rule2_directory_missing_returns_immediately_without_rule3_fallback() {
    let system_root = TempDirGuard::new("rule2_system_missing");
    let project_root = TempDirGuard::new("rule2_project_missing");
    // Even if filename exists elsewhere, rule2 must not fallback to rule3 scan.
    write_text_file(&project_root.path().join("summary.md"));

    let resolved = resolve_file_delivery_target(
        "去 missing_dir 找 summary.md",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule2DirNotFound
        ))
    );
}

#[test]
fn rule2_directory_and_file_found() {
    let system_root = TempDirGuard::new("rule2_system_found");
    let project_root = TempDirGuard::new("rule2_project_found");
    let target = project_root.path().join("docs/reports/summary.md");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "去 docs/reports 找 summary.md",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule2_directory_and_bare_stem_unique_extension_found() {
    let system_root = TempDirGuard::new("rule2_system_stem_unique");
    let project_root = TempDirGuard::new("rule2_project_stem_unique");
    let target = project_root.path().join("docs/reports/ABCD.txt");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "去 docs/reports 找 abcd",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule2_directory_and_bare_stem_multiple_extensions_requires_confirmation() {
    let system_root = TempDirGuard::new("rule2_system_stem_multi");
    let project_root = TempDirGuard::new("rule2_project_stem_multi");
    write_text_file(&project_root.path().join("docs/reports/abcd.txt"));
    write_text_file(&project_root.path().join("docs/reports/abcd.cpp"));

    let resolved = resolve_file_delivery_target(
        "去 docs/reports 找 abcd",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Candidates(vec![
            project_root
                .path()
                .join("docs/reports/abcd.cpp")
                .canonicalize()
                .expect("canonical abcd.cpp"),
            project_root
                .path()
                .join("docs/reports/abcd.txt")
                .canonicalize()
                .expect("canonical abcd.txt"),
        ]))
    );
}

#[test]
fn rule2_directory_found_but_file_missing() {
    let system_root = TempDirGuard::new("rule2_system_file_missing");
    let project_root = TempDirGuard::new("rule2_project_file_missing");
    fs::create_dir_all(project_root.path().join("docs/reports")).expect("create directory");

    let resolved = resolve_file_delivery_target(
        "去 docs/reports 找 summary.md",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule2FileNotFound
        ))
    );
}

#[test]
fn rule2_directory_fuzzy_name_requires_confirmation_instead_of_auto_delivery() {
    let system_root = TempDirGuard::new("rule2_system_fuzzy");
    let project_root = TempDirGuard::new("rule2_project_fuzzy");
    let target = project_root.path().join("docs/reports/日报_最终版.txt");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "去 docs/reports 找 最终版.txt",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Candidates(vec![target
            .canonicalize()
            .expect("canonical target"),]))
    );
}

#[test]
fn rule3_filename_only_scan_hits_under_project_root() {
    let system_root = TempDirGuard::new("rule3_system_hit");
    let project_root = TempDirGuard::new("rule3_project_hit");
    let target = project_root.path().join("docs/README.md");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "把 README.md 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule3_filename_only_bare_stem_unique_extension_resolves_directly() {
    let system_root = TempDirGuard::new("rule3_system_stem_unique");
    let project_root = TempDirGuard::new("rule3_project_stem_unique");
    let target = project_root.path().join("docs/ABCD.txt");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "把 abcd 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule3_filename_only_bare_stem_multiple_extensions_requires_confirmation() {
    let system_root = TempDirGuard::new("rule3_system_stem_multi");
    let project_root = TempDirGuard::new("rule3_project_stem_multi");
    write_text_file(&project_root.path().join("docs/abcd.txt"));
    write_text_file(&project_root.path().join("docs/abcd.cpp"));

    let resolved = resolve_file_delivery_target(
        "把 abcd 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Candidates(vec![
            project_root
                .path()
                .join("docs/abcd.cpp")
                .canonicalize()
                .expect("canonical abcd.cpp"),
            project_root
                .path()
                .join("docs/abcd.txt")
                .canonicalize()
                .expect("canonical abcd.txt"),
        ]))
    );
}

#[test]
fn rule3_filename_only_scan_falls_back_to_system_root() {
    let system_root = TempDirGuard::new("rule3_system_fallback");
    let project_root = TempDirGuard::new("rule3_project_fallback");
    let target = system_root.path().join("etc/demo.conf");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "把 demo.conf 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Resolved(
            target.canonicalize().expect("canonical target")
        ))
    );
}

#[test]
fn rule3_filename_only_fuzzy_name_requires_confirmation_instead_of_auto_delivery() {
    let system_root = TempDirGuard::new("rule3_system_fuzzy");
    let project_root = TempDirGuard::new("rule3_project_fuzzy");
    let target = project_root.path().join("docs/日报_最终版.txt");
    write_text_file(&target);

    let resolved = resolve_file_delivery_target(
        "把 最终版.txt 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Candidates(vec![target
            .canonicalize()
            .expect("canonical target"),]))
    );
}

#[test]
fn rule3_filename_only_fuzzy_name_returns_ranked_top3_candidates() {
    let system_root = TempDirGuard::new("rule3_system_fuzzy_top3");
    let project_root = TempDirGuard::new("rule3_project_fuzzy_top3");
    let c1 = project_root.path().join("docs/abcd_report.md");
    let c2 = project_root.path().join("docs/my_abcd.txt");
    let c3 = project_root.path().join("docs/x_abcd_log.txt");
    let c4 = project_root.path().join("docs/zz_abcd_backup.log");
    write_text_file(&c1);
    write_text_file(&c2);
    write_text_file(&c3);
    write_text_file(&c4);

    let resolved = resolve_file_delivery_target(
        "把 abcd 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::Candidates(vec![
            c1.canonicalize().expect("canonical c1"),
            c2.canonicalize().expect("canonical c2"),
            c3.canonicalize().expect("canonical c3"),
        ]))
    );
}

#[test]
fn rule3_filename_only_scan_not_found() {
    let system_root = TempDirGuard::new("rule3_system_not_found");
    let project_root = TempDirGuard::new("rule3_project_not_found");

    let resolved = resolve_file_delivery_target(
        "把 unknown_file_22781.md 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule3FileNotFound
        ))
    );
}

#[test]
fn rule3_filename_only_long_missing_name_does_not_match_short_substrings() {
    let system_root = TempDirGuard::new("rule3_system_missing_long_name");
    let project_root = TempDirGuard::new("rule3_project_missing_long_name");
    write_text_file(&project_root.path().join("rustclaw.service"));
    write_text_file(&project_root.path().join("README_file.txt"));

    let resolved = resolve_file_delivery_target(
        "把 definitely_missing_named_file_rustclaw_001.txt 发给我",
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule3FileNotFound
        ))
    );
}

#[test]
fn rule3_filename_only_missing_name_does_not_fallback_to_real_system_root_scan() {
    let project_root = TempDirGuard::new("rule3_project_missing_real_system_root");
    write_text_file(&project_root.path().join("rustclaw.service"));
    write_text_file(&project_root.path().join("README_file.txt"));

    let resolved = resolve_file_delivery_target(
        "把 definitely_missing_named_file_rustclaw_001.txt 发给我",
        std::path::Path::new("/"),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule3FileNotFound
        ))
    );
}

#[test]
fn rule3_filename_only_scan_rejects_when_scope_too_large() {
    let system_root = TempDirGuard::new("rule3_system_too_many");
    let project_root = TempDirGuard::new("rule3_project_too_many");
    for idx in 0..6 {
        write_text_file(&project_root.path().join(format!("f{idx}.txt")));
    }

    let resolved = resolve_file_delivery_target(
        "把 target.md 发给我",
        system_root.path(),
        project_root.path(),
        3,
        3,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule3ScanTooMany
        ))
    );
}

#[test]
fn rule3_filename_only_scan_respects_depth_limit() {
    let system_root = TempDirGuard::new("rule3_system_depth");
    let project_root = TempDirGuard::new("rule3_project_depth");
    let deep_target = project_root.path().join("a/b/c/deep.txt");
    write_text_file(&deep_target);

    let resolved = resolve_file_delivery_target(
        "把 deep.txt 发给我",
        system_root.path(),
        project_root.path(),
        1,
        200,
    );

    assert_eq!(
        resolved,
        Some(FileDeliveryTargetResolution::UserMessage(
            DeliveryMessageKind::Rule3FileNotFound
        ))
    );
}

// Directory lookup resolution rules.
#[test]
fn directory_rule_explicit_path_hits_system_root() {
    let system_root = TempDirGuard::new("dir_rule_system_hit");
    let project_root = TempDirGuard::new("dir_rule_project_hit");
    let dir = system_root.path().join("logs");
    fs::create_dir_all(&dir).expect("create logs");
    write_text_file(&dir.join("a.log"));

    let resolved = resolve_directory_target(
        DirectoryLookupInput::ExplicitPath {
            directory_path: "/logs".to_string(),
        },
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        DirectoryLookupResolution::Resolved(dir.canonicalize().expect("canonical logs"))
    );
}

#[test]
fn directory_rule_explicit_path_hits_project_root() {
    let system_root = TempDirGuard::new("dir_rule_system_project_hit");
    let project_root = TempDirGuard::new("dir_rule_project_hit");
    let dir = project_root.path().join("reports");
    fs::create_dir_all(&dir).expect("create reports");
    write_text_file(&dir.join("daily.txt"));

    let resolved = resolve_directory_target(
        DirectoryLookupInput::ExplicitPath {
            directory_path: "/reports".to_string(),
        },
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        DirectoryLookupResolution::Resolved(dir.canonicalize().expect("canonical reports"))
    );
}

#[test]
fn directory_rule_explicit_path_case_mismatch_hits_project_root() {
    let system_root = TempDirGuard::new("dir_rule_system_project_case_mismatch");
    let project_root = TempDirGuard::new("dir_rule_project_case_mismatch");
    let dir = project_root.path().join("Reports");
    fs::create_dir_all(&dir).expect("create reports");
    write_text_file(&dir.join("daily.txt"));

    let resolved = resolve_directory_target(
        DirectoryLookupInput::ExplicitPath {
            directory_path: "/reports".to_string(),
        },
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        DirectoryLookupResolution::Resolved(dir.canonicalize().expect("canonical reports"))
    );
}

#[test]
fn directory_rule_explicit_path_miss_both_roots() {
    let system_root = TempDirGuard::new("dir_rule_system_miss");
    let project_root = TempDirGuard::new("dir_rule_project_miss");

    let resolved = resolve_directory_target(
        DirectoryLookupInput::ExplicitPath {
            directory_path: "/missing_dir".to_string(),
        },
        system_root.path(),
        project_root.path(),
        3,
        200,
    );

    assert_eq!(
        resolved,
        DirectoryLookupResolution::UserMessage(DeliveryMessageKind::DirectoryBothRootsMiss)
    );
}

#[test]
fn directory_rule_name_hint_unique_hit() {
    let system_root = TempDirGuard::new("dir_rule_hint_unique_system");
    let project_root = TempDirGuard::new("dir_rule_hint_unique_project");
    let logs_dir = project_root.path().join("x/logs");
    fs::create_dir_all(&logs_dir).expect("create logs");

    let resolved = resolve_directory_target(
        DirectoryLookupInput::NameHint {
            directory_hint: "logs".to_string(),
        },
        system_root.path(),
        project_root.path(),
        4,
        300,
    );

    assert_eq!(
        resolved,
        DirectoryLookupResolution::Resolved(logs_dir.canonicalize().expect("canonical logs"))
    );
}

#[test]
fn directory_rule_name_hint_multiple_candidates_keep_top3() {
    let system_root = TempDirGuard::new("dir_rule_hint_multi_system");
    let project_root = TempDirGuard::new("dir_rule_hint_multi_project");
    fs::create_dir_all(system_root.path().join("a/logs")).expect("create a/logs");
    fs::create_dir_all(system_root.path().join("b/logs")).expect("create b/logs");
    fs::create_dir_all(project_root.path().join("c/logs")).expect("create c/logs");
    fs::create_dir_all(project_root.path().join("d/logs")).expect("create d/logs");

    let resolved = resolve_directory_target(
        DirectoryLookupInput::NameHint {
            directory_hint: "logs".to_string(),
        },
        system_root.path(),
        project_root.path(),
        4,
        500,
    );

    match resolved {
        DirectoryLookupResolution::MultipleCandidates(candidates) => {
            assert_eq!(candidates.len(), 3);
            assert!(candidates.iter().all(|p| p.is_absolute()));
        }
        other => panic!("expected multiple candidates, got {other:?}"),
    }
}

#[test]
fn directory_rule_name_hint_not_found() {
    let system_root = TempDirGuard::new("dir_rule_hint_not_found_system");
    let project_root = TempDirGuard::new("dir_rule_hint_not_found_project");

    let resolved = resolve_directory_target(
        DirectoryLookupInput::NameHint {
            directory_hint: "never_seen_dir_99273".to_string(),
        },
        system_root.path(),
        project_root.path(),
        4,
        300,
    );

    assert_eq!(
        resolved,
        DirectoryLookupResolution::UserMessage(DeliveryMessageKind::DirectoryBothRootsMiss)
    );
}

#[test]
fn directory_execution_resolution_finds_unique_directory_hint() {
    let project_root = TempDirGuard::new("dir_exec_hint_unique_project");
    let archive_dir = project_root.path().join("docs/archive");
    fs::create_dir_all(&archive_dir).expect("create archive dir");
    write_text_file(&archive_dir.join("one.txt"));

    let resolved = resolve_directory_locator_for_execution("archive", project_root.path(), 4, 300);

    assert_eq!(
        resolved,
        Some(DirectoryLocatorExecutionResolution::Resolved(
            archive_dir.canonicalize().expect("canonical archive")
        ))
    );
}

#[test]
fn directory_execution_resolution_returns_top3_for_ambiguous_hint() {
    let project_root = TempDirGuard::new("dir_exec_hint_multi_project");
    fs::create_dir_all(project_root.path().join("a/archive")).expect("create a/archive");
    fs::create_dir_all(project_root.path().join("b/archive")).expect("create b/archive");
    fs::create_dir_all(project_root.path().join("c/archive")).expect("create c/archive");
    fs::create_dir_all(project_root.path().join("d/archive")).expect("create d/archive");

    let resolved = resolve_directory_locator_for_execution("archive", project_root.path(), 4, 500);

    match resolved {
        Some(DirectoryLocatorExecutionResolution::MultipleCandidates(candidates)) => {
            assert_eq!(candidates.len(), 3);
            assert!(candidates.iter().all(|path| path.is_absolute()));
        }
        other => panic!("expected multiple candidates, got {other:?}"),
    }
}

#[test]
fn directory_listing_outputs_current_level_files_only_non_recursive() {
    let root = TempDirGuard::new("dir_listing_non_recursive");
    let dir = root.path().join("output");
    fs::create_dir_all(dir.join("nested")).expect("create nested");
    write_text_file(&dir.join("one.txt"));
    write_text_file(&dir.join("nested/two.txt"));

    let listed = list_directory_entries_for_user(&dir, 100);

    match listed {
        DirectoryEntriesListResult::FilePaths(paths) => {
            let rendered = paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>();
            assert_eq!(rendered.len(), 1);
            assert!(rendered[0].ends_with("/output/one.txt"));
        }
        other => panic!("expected file paths, got {other:?}"),
    }
}

#[test]
fn directory_listing_stops_when_entries_exceed_limit() {
    let root = TempDirGuard::new("dir_listing_too_many");
    let dir = root.path().join("bulk");
    fs::create_dir_all(&dir).expect("create bulk");
    for idx in 0..5 {
        write_text_file(&dir.join(format!("f{idx}.txt")));
    }

    let listed = list_directory_entries_for_user(&dir, 3);

    assert_eq!(
        listed,
        DirectoryEntriesListResult::UserMessage(DeliveryMessageKind::DirectoryEntriesTooMany)
    );
}

#[test]
fn directory_lookup_is_separated_from_file_delivery_cues() {
    assert_eq!(
        classify_directory_lookup_input("把 output 目录里的文件路径列出来"),
        None
    );
    assert_eq!(
        classify_directory_lookup_input("把 reports 目录下的 daily.md 发给我"),
        None
    );
}

#[test]
fn directory_lookup_parses_explicit_path_hint_from_cn_directory_query() {
    assert_eq!(
        classify_directory_lookup_input("找 /var/log 这个目录"),
        Some(DirectoryLookupInput::ExplicitPath {
            directory_path: "/var/log".to_string()
        })
    );
}

#[test]
fn directory_lookup_can_be_driven_by_llm_locator_hint_without_language_keywords() {
    let contract = contract_with_delivery_intent(OutputDeliveryIntent::DirectoryLookup, "项目资料");
    assert_eq!(
        resolve_directory_locator_input(&contract, "please do it", Path::new("/tmp")),
        Some(DirectoryLookupInput::NameHint {
            directory_hint: "项目资料".to_string()
        })
    );
}

#[test]
fn directory_lookup_uses_current_workspace_locator_kind_without_text_reparse() {
    let root = TempDirGuard::new("current_workspace_lookup");
    let contract = IntentOutputContract {
        delivery_intent: OutputDeliveryIntent::DirectoryLookup,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        ..IntentOutputContract::default()
    };
    assert_eq!(
        resolve_directory_locator_input(&contract, "please do it", root.path()),
        Some(DirectoryLookupInput::ExplicitPath {
            directory_path: root
                .path()
                .canonicalize()
                .expect("canonical root")
                .display()
                .to_string()
        })
    );
}

#[test]
fn batch_directory_delivery_uses_current_workspace_locator_kind_without_text_reparse() {
    let root = TempDirGuard::new("current_workspace_batch_delivery");
    let contract = IntentOutputContract {
        delivery_intent: OutputDeliveryIntent::DirectoryBatchFiles,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        ..IntentOutputContract::default()
    };
    assert_eq!(
        resolve_directory_locator_input(&contract, "please do it", root.path()),
        Some(DirectoryLookupInput::ExplicitPath {
            directory_path: root
                .path()
                .canonicalize()
                .expect("canonical root")
                .display()
                .to_string()
        })
    );
}

#[test]
fn directory_lookup_does_not_hijack_non_directory_file_operation() {
    assert_eq!(
        classify_directory_lookup_input("查看 /etc/hosts 文件内容"),
        None
    );
}

// CJK-oriented locator parsing and matching behavior.
#[test]
fn chinese_filename_candidates_are_extracted() {
    let out = extract_filename_candidates("把 测试文档.md 发给我，并且发一下 日报_最终版.txt");
    assert!(out.iter().any(|v| v == "测试文档.md"));
    assert!(out.iter().any(|v| v == "日报_最终版.txt"));
}

#[test]
fn ascii_bare_filename_stem_candidates_are_extracted_without_action_words() {
    let out = extract_bare_filename_stem_candidates("把 abcd 发给我，然后去 docs/reports 找 efgh");
    assert!(out.iter().any(|v| v == "abcd"));
    assert!(out.iter().any(|v| v == "efgh"));
    assert!(!out.iter().any(|v| v == "docs"));
}

#[test]
fn chinese_directory_name_hint_is_extracted() {
    assert_eq!(
        directory_lookup_input_from_hint("日志"),
        Some(DirectoryLookupInput::NameHint {
            directory_hint: "日志".to_string()
        })
    );
    assert_eq!(
        directory_lookup_input_from_hint("项目资料"),
        Some(DirectoryLookupInput::NameHint {
            directory_hint: "项目资料".to_string()
        })
    );
}

#[test]
fn inline_ascii_directory_name_hint_is_extracted_from_request() {
    assert_eq!(
        classify_directory_lookup_input("发 document 目录下最后一个"),
        Some(DirectoryLookupInput::NameHint {
            directory_hint: "document".to_string()
        })
    );
    assert_eq!(
        classify_directory_lookup_input("列出 logs directory 下面前 5 个文件"),
        Some(DirectoryLookupInput::NameHint {
            directory_hint: "logs".to_string()
        })
    );
}

#[test]
fn chinese_directory_and_file_pair_is_extracted() {
    assert_eq!(
        extract_directory_and_file_pair("在 项目资料 目录下找 日报.md"),
        Some(("项目资料".to_string(), "日报.md".to_string()))
    );
}

#[test]
fn chinese_path_fragments_are_extracted() {
    let out = extract_explicit_file_path_candidates(
        "把 /home/guagua/资料/日报.md 发给我，然后去 ./输出目录/报告.txt 看看",
    );
    assert!(out.iter().any(|v| v == "/home/guagua/资料/日报.md"));
    assert!(out.iter().any(|v| v == "./输出目录/报告.txt"));
}

#[test]
fn chinese_directory_name_is_matchable_in_directory_scan() {
    let root = TempDirGuard::new("cn_dir_scan");
    fs::create_dir_all(root.path().join("项目资料/日志")).expect("create cn dirs");

    let out = collect_directory_candidates(root.path(), "项目资料", 3, 100, true);
    assert_eq!(out.len(), 1);
    assert!(out[0].ends_with("项目资料"));
}

#[test]
fn chinese_filename_matches_non_recursive_lookup() {
    let root = TempDirGuard::new("cn_file_non_recursive");
    let dir = root.path().join("输出目录");
    fs::create_dir_all(&dir).expect("create dir");
    let target = dir.join("日报.md");
    write_text_file(&target);

    let out = find_file_in_directory_non_recursive(&dir, "日报.md");
    assert_eq!(
        out,
        DirectoryFileLookupResult::Found(target.canonicalize().expect("canonical target"))
    );
}

#[test]
fn chinese_filename_supports_normalized_contains_match_non_recursive() {
    let root = TempDirGuard::new("cn_file_contains_non_recursive");
    let dir = root.path().join("输出目录");
    fs::create_dir_all(&dir).expect("create dir");
    let target = dir.join("日报_最终版.txt");
    write_text_file(&target);

    let out = find_file_in_directory_non_recursive(&dir, "最终版.txt");
    assert_eq!(
        out,
        DirectoryFileLookupResult::Candidates(vec![target
            .canonicalize()
            .expect("canonical target")])
    );
}

#[test]
fn chinese_filename_matches_project_root_scan() {
    let root = TempDirGuard::new("cn_file_scan");
    let target = root.path().join("项目资料/日报.md");
    write_text_file(&target);

    let out = scan_filename_matches_with_limit(root.path(), "日报.md", 3, 100);
    assert_eq!(
        out,
        FilenameScanResult::Found(target.canonicalize().expect("canonical target"))
    );
}

#[test]
fn chinese_filename_supports_normalized_contains_match_project_scan() {
    let root = TempDirGuard::new("cn_file_scan_contains");
    let target = root.path().join("项目资料/日报_最终版.txt");
    write_text_file(&target);

    let out = scan_filename_matches_with_limit(root.path(), "最终版.txt", 3, 100);
    assert_eq!(
        out,
        FilenameScanResult::Candidates(vec![target.canonicalize().expect("canonical target")])
    );
}

// Batch directory delivery behavior stays in the file-delivery submodule.
#[test]
fn batch_directory_delivery_formats_multiline_file_tokens() {
    let root = TempDirGuard::new("batch_tokens");
    let a = root.path().join("a.txt");
    let b = root.path().join("b.txt");
    write_text_file(&a);
    write_text_file(&b);

    let mut files = vec![
        a.canonicalize().expect("canonical a"),
        b.canonicalize().expect("canonical b"),
    ];
    files.sort();
    let text = format_batch_delivery_tokens(&files, None);
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|line| line.starts_with("FILE:")));
}

#[test]
fn batch_directory_delivery_intent_is_distinct_from_single_file_delivery() {
    assert_eq!(
        classify_batch_directory_delivery_input("把这个文件夹下面的文件发我"),
        None
    );
    assert_eq!(
        classify_batch_directory_delivery_input("把 reports 目录下的 daily.md 发给我"),
        None
    );
}

#[test]
fn batch_directory_delivery_only_sends_current_level_and_adds_child_dir_hint() {
    let root = TempDirGuard::new("batch_current_level");
    let dir = root.path().join("output");
    fs::create_dir_all(dir.join("nested")).expect("create nested");
    let current = dir.join("one.txt");
    let nested = dir.join("nested/two.txt");
    write_text_file(&current);
    write_text_file(&nested);

    let listed = list_current_level_files_for_delivery(&dir, 200);
    let CurrentLevelDeliveryEntriesResult::Ready(entries) = listed else {
        panic!("expected ready entries");
    };
    assert!(entries.has_child_dirs);
    assert_eq!(entries.files.len(), 1);
    assert!(entries.files[0]
        .to_string_lossy()
        .ends_with("/output/one.txt"));
    let resolved = build_batch_directory_delivery_response(
        entries,
        "该目录当前层没有可发送的文件",
        "这个目录下还有其他子目录，如需继续发送，请提供更准确路径",
    );
    match resolved {
        BatchDirectoryDeliveryResolution::FileTokens(text) => {
            assert!(text.contains("FILE:"));
            assert!(!text.contains("nested/two.txt"));
            assert!(text.contains("这个目录下还有其他子目录"));
        }
        other => panic!("expected file tokens, got {other:?}"),
    }
}

#[test]
fn batch_directory_delivery_returns_no_sendable_files_message_when_current_level_has_no_files() {
    let entries = CurrentLevelDeliveryEntries {
        files: Vec::new(),
        has_child_dirs: false,
    };
    let resolved = build_batch_directory_delivery_response(
        entries,
        "该目录当前层没有可发送的文件",
        "这个目录下还有其他子目录，如需继续发送，请提供更准确路径",
    );
    assert_eq!(
        resolved,
        BatchDirectoryDeliveryResolution::UserMessage("该目录当前层没有可发送的文件".to_string())
    );
}

#[test]
fn batch_directory_delivery_no_files_with_child_dirs_appends_hint() {
    let entries = CurrentLevelDeliveryEntries {
        files: Vec::new(),
        has_child_dirs: true,
    };
    let resolved = build_batch_directory_delivery_response(
        entries,
        "该目录当前层没有可发送的文件",
        "这个目录下还有其他子目录，如需继续发送，请提供更准确路径",
    );
    match resolved {
        BatchDirectoryDeliveryResolution::UserMessage(text) => {
            assert!(text.contains("该目录当前层没有可发送的文件"));
            assert!(text.contains("这个目录下还有其他子目录"));
        }
        other => panic!("expected user message, got {other:?}"),
    }
}

#[test]
fn batch_directory_delivery_stops_when_entries_exceed_limit() {
    let root = TempDirGuard::new("batch_too_many");
    let dir = root.path().join("bulk");
    fs::create_dir_all(&dir).expect("create bulk");
    for idx in 0..6 {
        write_text_file(&dir.join(format!("f{idx}.txt")));
    }

    let listed = list_current_level_files_for_delivery(&dir, 3);
    assert_eq!(
        listed,
        CurrentLevelDeliveryEntriesResult::UserMessage(
            DeliveryMessageKind::DirectoryEntriesTooMany
        )
    );
}

#[test]
fn batch_directory_delivery_directory_not_found_does_not_enter_file_tokens() {
    let system_root = TempDirGuard::new("batch_dir_miss_system");
    let project_root = TempDirGuard::new("batch_dir_miss_project");
    let locator = directory_lookup_input_from_hint("missing_dir").expect("batch locator");
    let resolved =
        resolve_directory_target(locator, system_root.path(), project_root.path(), 3, 200);
    assert_eq!(
        resolved,
        DirectoryLookupResolution::UserMessage(DeliveryMessageKind::DirectoryBothRootsMiss)
    );
}
