use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    agent_context_allows_observed_output_language_fallback,
    attach_deterministic_observed_execution_status_answer,
    attach_execution_recipe_closeout_to_delivery, attach_execution_summary_to_delivery,
    auto_requested_success_marker, backfill_delivery_from_last_outputs,
    build_execution_summary_message, build_execution_summary_messages,
    compare_paths_size_ratio_answer, content_evidence_step_failure_answer,
    content_evidence_terminal_respond_is_contractual_answer,
    delivery_contract_suppresses_execution_summary, delivery_is_content_answer_candidate,
    deterministic_missing_observed_target_answer, deterministic_observed_execution_status_answer,
    deterministic_structured_file_validation_from_read_range, direct_config_edit_observed_answer,
    direct_db_basic_observed_answer, direct_directory_purpose_summary_from_size_facts,
    direct_file_token_from_observed_auto_locator_filename,
    direct_file_token_from_observed_inventory, direct_log_tail_status_answer,
    direct_non_builtin_skill_raw_answer, direct_path_from_active_bound_inventory,
    direct_publishable_observed_answer, direct_quantity_comparison_from_compare_paths,
    direct_rustclaw_config_risk_answer, direct_scalar_observed_answer,
    direct_structured_observed_answer,
    discard_non_answer_separator_delivery_for_broad_structured_read,
    discard_raw_passthrough_delivery_when_structured_answer_available,
    ensure_requested_success_marker_visible, execution_recipe_closeout_note,
    final_answer_text_from_delivery, finalize_loop_reply, finalizer_requires_clarify,
    has_missing_file_search_evidence, latest_file_delivery_observation_is_missing,
    looks_like_raw_command_snapshot, looks_like_structured_machine_output,
    markdown_heading_from_read_output, missing_requested_success_marker,
    normalize_file_token_delivery_from_auto_locator,
    normalize_file_token_delivery_from_observed_paths,
    observed_execution_without_publishable_delivery_outcome,
    observed_execution_without_publishable_delivery_reply, observed_synthesis_unavailable_reply,
    path_batch_size_comparison_answer, prefer_observed_answer_for_exact_contract,
    replace_delivery_with_deterministic_directory_purpose_answer,
    replace_delivery_with_deterministic_execution_failed_step_answer,
    replace_delivery_with_deterministic_observed_execution_status_answer,
    replace_delivery_with_deterministic_rustclaw_config_risk_answer,
    replace_delivery_with_direct_log_tail_status_answer,
    replace_delivery_with_latest_tail_read_range_answer,
    replace_delivery_with_observed_markdown_heading_scalar,
    resolve_file_token_from_auto_locator_answer, should_attach_execution_summary,
    should_drop_passthrough_delivery_for_content_evidence,
    should_return_missing_file_delivery_reply, should_try_observed_output_language_fallback,
    successful_delivery_final_status, verify_summary_requires_resume_confirmation,
};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    AgentRuntimeConfig, AppState, ClaimedTask, IntentOutputContract, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
    ScheduleKind, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};

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
            "clawd_loop_finalize_{prefix}_{}_{}",
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

fn claimed_task(task_id: &str) -> ClaimedTask {
    ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn test_state() -> AppState {
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(
                    ["crypto".to_string(), "stock".to_string()]
                        .into_iter()
                        .collect::<HashSet<_>>(),
                ),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn verify_summary(
    mode: crate::verifier::VerifyMode,
) -> crate::task_journal::TaskJournalVerifySummary {
    crate::task_journal::TaskJournalVerifySummary {
        mode,
        approved: true,
        needs_confirmation: true,
        ..Default::default()
    }
}

fn finalizer_summary(
    disposition: crate::finalize::FinalizerDisposition,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(disposition),
        ..Default::default()
    }
}

fn scalar_route_result() -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "extract scalar".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: Default::default(),
            semantic_kind: Default::default(),
            locator_hint: "package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn free_route_result() -> RouteResult {
    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route
}

fn push_raw_plan_text(loop_state: &mut crate::agent_engine::LoopState, raw_plan_text: &str) {
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                steps: Vec::new(),
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: raw_plan_text.to_string(),
            }),
            verify_result: None,
        });
}

#[test]
fn compare_paths_size_ratio_answer_computes_ratio_from_structured_output() {
    let answer = compare_paths_size_ratio_answer(
        r#"{"action":"compare_paths","left":{"path":"Cargo.lock","size_bytes":121647},"right":{"path":"Cargo.toml","size_bytes":2606},"comparison":{"same_size":false}}"#,
        false,
    )
    .expect("ratio answer");

    assert!(answer.contains("Cargo.lock"));
    assert!(answer.contains("Cargo.toml"));
    assert!(answer.contains("46.68"));
}

#[test]
fn path_batch_size_comparison_answer_picks_largest_structured_size() {
    let answer = path_batch_size_comparison_answer(
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","size_bytes":2606},"path":"Cargo.toml"},{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","size_bytes":121647},"path":"Cargo.lock"}]}"#,
        false,
    )
    .expect("size comparison answer");

    assert!(answer.contains("Cargo.lock"));
    assert!(answer.contains("更大"));
    assert!(answer.contains("46.68"));
}

#[test]
fn directory_purpose_summary_from_size_facts_picks_largest_file() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"names":["contract_repair_judge.schema.json","intent_normalizer.schema.json"]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/contract_repair_judge.schema.json","size_bytes":6112},"path":"prompts/schemas/contract_repair_judge.schema.json"},{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":13124},"path":"prompts/schemas/intent_normalizer.schema.json"}]}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "prompts/schemas".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) = direct_directory_purpose_summary_from_size_facts(
        &state,
        "告诉我哪个 schema 最大",
        &loop_state,
        Some(&ctx),
    )
    .expect("directory purpose size facts answer");

    assert!(answer.contains("intent_normalizer.schema.json"));
    assert!(answer.contains("13124"));
    assert!(!answer.contains("contract_repair_judge.schema.json（6112"));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn directory_purpose_summary_replaces_wrong_synthesis_largest_file() {
    let state = test_state();
    let task = claimed_task("task-directory-purpose-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("最大的是 contract_repair_judge.schema.json（6112 字节）。".to_string());
    loop_state.last_user_visible_respond =
        Some("最大的是 contract_repair_judge.schema.json（6112 字节）。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/contract_repair_judge.schema.json","size_bytes":6112},"path":"prompts/schemas/contract_repair_judge.schema.json"},{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":13124},"path":"prompts/schemas/intent_normalizer.schema.json"}]}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "prompts/schemas".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_directory_purpose_answer(
            &state,
            &task,
            "告诉我哪个 schema 最大",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("intent_normalizer.schema.json"));
    assert!(answer.contains("13124"));
    assert!(!answer.contains("contract_repair_judge.schema.json（6112"));
    assert!(summary.is_some());
}

#[test]
fn direct_quantity_comparison_from_count_inventory_prefers_total_size() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"target","resolved_path":"/tmp/repo/target","size_bytes":4096},"path":"target"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"count_inventory","path":"target","resolved_path":"/tmp/repo/target","recursive":true,"counts":{"total":129116,"files":100000,"dirs":29116,"total_size_bytes":57268736832}}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "target".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "看一下 target 大概多大",
        &loop_state,
        Some(&ctx),
    )
    .expect("count_inventory total size answer");

    assert!(answer.contains("57268736832"));
    assert!(answer.contains("53.3 GiB"));
    assert!(answer.contains("129116"));
    assert!(!answer.contains('\n'));
    assert!(answer.starts_with("path=target size.bytes=57268736832"));
    assert!(!answer.trim().eq("129116"));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn direct_quantity_comparison_defers_two_count_inventory_totals_to_synthesis() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","path":"scripts/nl_tests/fixtures/device_local/docs","recursive":false,"counts":{"total":3,"files":2,"dirs":1,"total_size_bytes":425}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"count_inventory","path":"scripts/nl_tests/fixtures/device_local/logs","recursive":false,"counts":{"total":2,"files":2,"dirs":0,"total_size_bytes":2698}}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let answer = direct_quantity_comparison_from_compare_paths(
        &state,
        "先数 docs 直接子项数量，再数 logs 直接子项数量，最后一句中文说哪个更多",
        &loop_state,
        Some(&ctx),
    );

    assert!(
        answer.is_none(),
        "multi-target count comparisons need synthesized language, got {answer:?}"
    );
}

#[test]
fn direct_quantity_comparison_from_ranked_inventory_outputs_name_size_lines() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"logs","resolved_path":"/tmp/repo/logs","sort_by":"size_desc","entries":[{"kind":"file","name":"large.log","size_bytes":900},{"kind":"file","name":"small.log","size_bytes":12}],"counts":{"files":2,"total":2}}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "列出 logs 目录下最大的 2 个文件",
        &loop_state,
        Some(&ctx),
    )
    .expect("ranked inventory answer");

    assert_eq!(answer, "large.log 900\nsmall.log 12");
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn quantity_comparison_replaces_synthesis_count_with_total_size_answer() {
    let state = test_state();
    let task = claimed_task("task-quantity-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("129116，当前范围内共有 129116 个项目。".to_string());
    loop_state.last_user_visible_respond =
        Some("129116，当前范围内共有 129116 个项目。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","path":"target","resolved_path":"/tmp/repo/target","recursive":true,"counts":{"total":129116,"files":100000,"dirs":29116,"total_size_bytes":57268736832}}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "target".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        super::replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "看一下 target 大概多大",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("57268736832"));
    assert!(answer.contains("53.3 GiB"));
    assert!(!answer.contains('\n'));
    assert!(!answer.trim().starts_with("129116"));
    assert!(summary.is_some());
}

#[test]
fn quantity_comparison_preserves_synthesis_with_both_path_sizes() {
    let state = test_state();
    let task = claimed_task("task-quantity-preserve-synthesis");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    let synthesized = "README.md（46320 字节）比 README.zh-CN.md（39733 字节）更大。原因是英文文档通常比中文文档占用更多字节，同等内容的英文表达往往比中文更冗长。";
    loop_state.delivery_messages.push(synthesized.to_string());
    loop_state.last_user_visible_respond = Some(synthesized.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/repo/README.md","size_bytes":46320},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"README.zh-CN.md","resolved_path":"/tmp/repo/README.zh-CN.md","size_bytes":39733},"path":"README.zh-CN.md"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        synthesized,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md|README.zh-CN.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        super::replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "比较 README.md 和 README.zh-CN.md 哪个更大，再解释原因",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![synthesized.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesized)
    );
    assert!(summary.is_some());
}

#[test]
fn direct_quantity_comparison_from_compare_paths_recovers_after_synthesis_failure() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"compare_paths","left":{"path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","kind":"file","size_bytes":121647},"right":{"path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","kind":"file","size_bytes":2606},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":119041,"left_newer":false,"same_content":false}}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock|Cargo.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured ratio fallback");

    assert!(answer.contains("46.68"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_from_path_batch_facts_recovers_after_synthesis_failure() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2606},"path":"Cargo.toml"},{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121647},"path":"Cargo.lock"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml|Cargo.lock".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured path facts size fallback");

    assert!(answer.contains("Cargo.lock"));
    assert!(answer.contains("46.68"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_scalar_shape_returns_ratio_not_byte_delta() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121800},"path":"Cargo.lock"},{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2639},"path":"Cargo.toml"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock|Cargo.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured scalar ratio fallback");

    assert!(answer.contains("Cargo.lock"));
    assert!(answer.contains("Cargo.toml"));
    assert!(answer.contains("46.15"));
    assert!(answer.contains("更大"));
    assert!(!answer.contains("is larger"));
    assert!(!answer.contains("119161"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_uses_original_request_language_over_scaffold() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121800},"path":"Cargo.lock"},{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2639},"path":"Cargo.toml"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock|Cargo.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(
            "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍".to_string(),
        ),
        ..Default::default()
    };

    let (answer, _) = direct_quantity_comparison_from_compare_paths(
        &state,
        "### MEMORY_USE_POLICY\nCargo.lock Cargo.toml lock toml ratio",
        &loop_state,
        Some(&ctx),
    )
    .expect("contextual language fallback");

    assert!(answer.contains("更大"));
    assert!(!answer.contains("is larger"));
}

#[test]
fn direct_quantity_comparison_strict_shape_returns_byte_delta() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/README.md","size_bytes":29191},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"AGENTS.md","resolved_path":"/tmp/AGENTS.md","size_bytes":20744},"path":"AGENTS.md"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md|AGENTS.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "Compare README.md and AGENTS.md by file size.\n[CONTRACT_TEST_HINT]\nselector_answer_style=delta_only\n[/CONTRACT_TEST_HINT]",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured strict delta fallback");

    assert_eq!(answer, "README.md: 8447 bytes");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_contract_selector_returns_larger_with_sizes() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"compare_paths","left":{"path":"release_checklist.md","resolved_path":"/tmp/release_checklist.md","kind":"file","size_bytes":153},"right":{"path":"package.json","resolved_path":"/tmp/package.json","kind":"file","size_bytes":246},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":-93,"left_newer":false,"same_content":false}}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, _summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较两个文件大小\n[CONTRACT_TEST_HINT]\nselector_answer_style=larger_with_sizes\n[/CONTRACT_TEST_HINT]",
        &loop_state,
        Some(&ctx),
    )
    .expect("selector should force complete comparison answer");

    assert!(answer.contains("package.json"), "answer: {answer}");
    assert!(answer.contains("246"), "answer: {answer}");
    assert!(answer.contains("release_checklist.md"), "answer: {answer}");
    assert!(answer.contains("153"), "answer: {answer}");
    assert!(
        !answer.contains("package.json：93 字节"),
        "answer: {answer}"
    );
}

fn plan_result_with_steps(steps: Vec<crate::PlanStep>) -> crate::PlanResult {
    crate::PlanResult {
        goal: "test goal".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps,
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    }
}

fn plan_result_with_raw_text(raw_plan_text: &str) -> crate::PlanResult {
    crate::PlanResult {
        raw_plan_text: raw_plan_text.to_string(),
        ..plan_result_with_steps(Vec::new())
    }
}

fn ok_step_result(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn direct_structured_observed_answer_defers_implicit_metadata_path_facts() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"tmp/test_bundle.zip","resolved_path":"/tmp/test_bundle.zip","size_bytes":272,"modified_ts":1776352013},"path":"/tmp/test_bundle.zip"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_structured_observed_answer(Some(&state), &loop_state, Some(&ctx)).is_none());
    assert!(super::latest_path_batch_facts_has_implicit_metadata_fields(
        &loop_state
    ));
}

#[test]
fn direct_db_basic_observed_answer_uses_latest_rows_after_synthesis_failure() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 1,
        finished_at: 2,
    });
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "db_basic",
        r#"{"columns":["id","name"],"rows":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]}"#,
    ));

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, summary) = direct_db_basic_observed_answer(
        &state,
        "Read id and name from users limit 2.",
        &loop_state,
        Some(&ctx),
    )
    .expect("db rows fallback");

    assert!(answer.contains("id: 1"));
    assert!(answer.contains("name: Alice"));
    assert!(answer.contains("id: 2"));
    assert!(answer.contains("name: Bob"));
    assert!(!answer.contains("orders"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_db_basic_observed_answer_counts_rows_for_scalar_count_contract() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
    ));

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, summary) = direct_db_basic_observed_answer(
        &state,
        "统计 SQLite 数据库的表数量，只输出数字",
        &loop_state,
        Some(&ctx),
    )
    .expect("scalar count fallback");

    assert_eq!(answer, "3");
    assert_eq!(summary.format_ok, Some(true));
}

#[test]
fn direct_structured_observed_answer_defers_when_plan_requested_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# Device Local Fixture\n2|\n3|This directory contains stable local files for tests."}"#,
    ));
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read then summarize".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_raw_text(
                r#"{"steps":[{"type":"call_tool","tool":"fs_basic"},{"type":"synthesize_answer","evidence_refs":["last_output"]}]}"#,
            )),
            verify_result: None,
        });
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/README.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_structured_observed_answer(None, &loop_state, Some(&ctx)).is_none());
}

#[test]
fn direct_scalar_observed_answer_extracts_markdown_heading_from_read_range() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist","path":"release_checklist.md"}"#,
    ));
    let route = scalar_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, _) =
        direct_scalar_observed_answer(None, &loop_state, Some(&ctx)).expect("heading answer");

    assert_eq!(answer, "Release Checklist");
    assert!(!should_attach_execution_summary(
        &loop_state,
        Some(&ctx),
        Some("Read the note file title and output only the title.")
    ));

    let mut route = scalar_route_result();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let (answer, _) =
        direct_scalar_observed_answer(None, &loop_state, Some(&ctx)).expect("heading answer");
    assert_eq!(answer, "Release Checklist");
    assert!(!should_attach_execution_summary(
        &loop_state,
        Some(&ctx),
        Some("Read the note file title and output only the title.")
    ));
}

#[test]
fn markdown_heading_direct_scalar_defers_when_read_evidence_has_body() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    assert!(markdown_heading_from_read_output(
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#
    )
    .is_none());
}

#[test]
fn direct_scalar_observed_answer_skips_separator_markdown_heading() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# =========================\n2|# Image Edit","path":"configs/image.toml"}"#,
    ));
    let route = scalar_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, _) =
        direct_scalar_observed_answer(None, &loop_state, Some(&ctx)).expect("heading answer");
    assert_eq!(answer, "Image Edit");
}

#[test]
fn execution_summary_suppressed_for_grounded_content_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|{\n2|  \"type\": \"object\",\n3|  \"additionalProperties\": false\n4|}","path":"prompts/schemas/direct_answer_gate.schema.json"}"#,
    ));
    loop_state.delivery_messages.push(
        "`additionalProperties: false` makes future schema extension more brittle.".to_string(),
    );
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "prompts/schemas/direct_answer_gate.schema.json".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };

    assert!(delivery_contract_suppresses_execution_summary(
        &loop_state,
        Some(&ctx),
        &loop_state.delivery_messages
    ));
    assert!(build_execution_summary_messages(
        &loop_state,
        Some(&ctx),
        Some("Check the schema risks briefly.")
    )
    .is_empty());
}

#[test]
fn observed_markdown_heading_scalar_replaces_repaired_strict_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.route_reason =
        "llm_semantic_contract_repair:malformed_contract_repairs_needed_but_conservative_route_valid"
            .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "note file".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Release Checklist".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["# Release Checklist".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["# Release Checklist".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_keeps_locatorless_strict_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Release Checklist".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["# Release Checklist".to_string()]);
    assert!(summary.is_none());
}

#[test]
fn observed_markdown_heading_scalar_replaces_direct_answer_gate_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.route_reason =
        "executionless_route_downgraded_to_direct_answer; direct_answer_gate_execute".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Service Notes".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["# Service Notes".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["# Service Notes".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_replaces_one_sentence_locator_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["Service Notes".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["Service Notes".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["Service Notes".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_suppresses_summary_for_free_locator_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["Service Notes".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["Service Notes".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["Service Notes".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_reduces_strict_observed_markdown_body() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(
        delivery,
        vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()]
    );
    assert!(summary.is_none());
}

#[test]
fn observed_markdown_heading_scalar_keeps_free_observed_markdown_body() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(
        delivery,
        vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()]
    );
    assert!(summary.is_none());
}

#[test]
fn direct_structured_observed_answer_keeps_passthrough_without_synthesis_plan() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"1|[app]\n2|name = \"fixture\""}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/config.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, _) = direct_structured_observed_answer(None, &loop_state, Some(&ctx))
        .expect("direct passthrough without synthesis plan");

    assert_eq!(answer, "[app]\nname = \"fixture\"");
}

#[test]
fn broad_structured_read_drops_separator_and_validates_file() {
    let state = test_state();
    let path = std::env::temp_dir().join(format!(
        "rustclaw_structured_validation_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[memory]\nconfig_path = \"configs/memory.toml\"\n")
        .expect("write temp toml");
    let path_text = path.to_string_lossy().to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec![
        "=============================================================================".to_string(),
    ];
    loop_state.last_user_visible_respond = Some(
        "=============================================================================".to_string(),
    );
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "read_range",
            "mode": "head",
            "requested_n": 120,
            "path": path_text,
            "resolved_path": path_text,
            "excerpt": "1|# ============================================================================="
        })
        .to_string(),
    ));

    assert!(
        discard_non_answer_separator_delivery_for_broad_structured_read("task", &mut loop_state)
    );
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.last_user_visible_respond.is_none());

    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, summary) = deterministic_structured_file_validation_from_read_range(
        &state,
        "Vérifie seulement si ce fichier est un TOML valide.",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured validation fallback");
    assert!(
        answer.contains("toml")
            && (answer.contains("解析成功") || answer.contains("parsed successfully")),
        "answer: {answer}"
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn broad_structured_read_validation_does_not_replace_directory_summary() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"head","path":"UI/package.json","resolved_path":"UI/package.json","excerpt":"1|{\n2|  \"name\": \"react-example\"\n3|}"}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(deterministic_structured_file_validation_from_read_range(
        &state,
        "Summarize the directory and use the package name as context.",
        &loop_state,
        Some(&ctx),
    )
    .is_none());
}

fn err_step_result(step_id: &str, skill: &str, error: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(error.to_string()),
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn direct_config_edit_observed_answer_summarizes_apply_validate_readback() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"plan_config_change","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","new_value":true,"would_change":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_edit",
        r#"{"action":"apply_config_change","applied":true,"path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","new_value":true,"validated":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "config_edit",
        r#"{"action":"validate_config","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","valid":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "config_edit",
        r#"{"action":"read_back","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","exists":true,"value":true,"value_text":"true"}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "把 config_edit_nl_smoke 开关打开，然后验证并读回",
        &loop_state,
    )
    .expect("config_edit structured answer");

    assert!(answer.contains("配置已更新"));
    assert!(answer.contains("skills.skill_switches.config_edit_nl_smoke"));
    assert!(answer.contains("true"));
    assert!(answer.contains("验证通过"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_config_edit_observed_answer_summarizes_guard_config() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"guard_config","path":"configs/config.toml","risk_count":2,"risks":["llm.minimax.api_key looks like a real secret","tools.allow_sudo=true"]}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "检查 RustClaw 主配置有没有明显风险，不能泄露任何密钥值",
        &loop_state,
    )
    .expect("config_edit guard answer");

    assert!(answer.contains("configs/config.toml"));
    assert!(answer.contains("2"));
    assert!(answer.contains("llm.minimax.api_key looks like a real secret"));
    assert!(answer.contains("tools.allow_sudo=true"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_config_edit_observed_answer_accepts_config_basic_guard_config() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"guard_config","path":"configs/config.toml","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"]}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "检查 RustClaw 主配置有没有明显风险，不能泄露任何密钥值",
        &loop_state,
    )
    .expect("config_basic guard answer");

    assert!(answer.contains("configs/config.toml"));
    assert!(answer.contains("2"));
    assert!(answer.contains("tools.allow_sudo=true"));
    assert!(answer.contains("tools.allow_path_outside_workspace=true"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_rustclaw_config_risk_answer_uses_structured_field_values() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_fields","path":"/home/guagua/rustclaw/configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","count":6,"results":[{"field_path":"tools.allow","resolved_field_path":"tools.allow","exists":true,"value":["*"],"value_text":"[\"*\"]"},{"field_path":"tools.allow_sudo","resolved_field_path":"tools.allow_sudo","exists":true,"value":true,"value_text":"true"},{"field_path":"tools.allow_path_outside_workspace","resolved_field_path":"tools.allow_path_outside_workspace","exists":true,"value":true,"value_text":"true"},{"field_path":"self_extension.enabled","resolved_field_path":"self_extension.enabled","exists":true,"value":false,"value_text":"false"},{"field_path":"worker.task_timeout_seconds","resolved_field_path":"worker.task_timeout_seconds","exists":true,"value":3600,"value_text":"3600"},{"field_path":"server.listen","resolved_field_path":"server.listen","exists":true,"value":"0.0.0.0:8787","value_text":"0.0.0.0:8787"}]}"#,
    ));

    let (answer, summary) = direct_rustclaw_config_risk_answer(
        &state,
        "configs/config.toml の RustClaw 設定リスクを確認し、重要な点だけ答えて。",
        &loop_state,
    )
    .expect("structured config risk answer");

    assert!(answer.contains("Found 4 config risk(s)"));
    assert!(answer.contains("tools.allow=[\"*\"]"));
    assert!(answer.contains("tools.allow_sudo=true"));
    assert!(answer.contains("tools.allow_path_outside_workspace=true"));
    assert!(answer.contains("server.listen=\"0.0.0.0:8787\""));
    assert!(!answer.contains("self_extension.enabled=true"));
    assert!(!answer.contains("86400"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn rustclaw_config_risk_replacement_drops_ungrounded_synthesis() {
    let state = test_state();
    let task = claimed_task("task-config-risk-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("self_extension.enabled=true and worker.task_timeout_seconds=86400".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_fields","path":"configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","results":[{"field_path":"tools.allow_sudo","resolved_field_path":"tools.allow_sudo","exists":true,"value":true},{"field_path":"tools.allow_path_outside_workspace","resolved_field_path":"tools.allow_path_outside_workspace","exists":true,"value":true},{"field_path":"self_extension.enabled","resolved_field_path":"self_extension.enabled","exists":true,"value":false},{"field_path":"worker.task_timeout_seconds","resolved_field_path":"worker.task_timeout_seconds","exists":true,"value":3600}]}"#,
    ));
    let mut summary = None;

    assert!(replace_delivery_with_deterministic_rustclaw_config_risk_answer(
        &state,
        &task,
        "Check configs/config.toml for RustClaw configuration risks and list only important findings.",
        &mut loop_state,
        &mut summary,
    ));

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("tools.allow_sudo=true"));
    assert!(answer.contains("tools.allow_path_outside_workspace=true"));
    assert!(!answer.contains("self_extension.enabled=true"));
    assert!(!answer.contains("86400"));
    assert!(summary.is_some());
}

#[tokio::test]
async fn finalize_loop_reply_uses_config_edit_observed_answer_after_synthesis_failure() {
    let state = test_state();
    let task = claimed_task("task-config-edit-fallback");
    let mut loop_state = crate::agent_engine::LoopState::new(5);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"apply_config_change","applied":true,"path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","new_value":true,"validated":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_edit",
        r#"{"action":"validate_config","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","valid":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "config_edit",
        r#"{"action":"read_back","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","exists":true,"value":true,"value_text":"true"}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_4",
        "synthesize_answer",
        "synthesis failed",
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 config_edit_nl_smoke 开关打开，然后验证并读回",
        loop_state,
        None,
    )
    .await
    .expect("finalize should succeed");

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("配置已更新"));
    assert!(reply.text.contains("验证通过"));
    assert!(!reply.text.contains("没能整理成可靠结论"));
}

#[test]
fn execution_summary_attaches_before_final_delivery_without_changing_final_text() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list recent logs".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "ls -t logs | head -2"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "model_io.log\nact_plan.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["这更像运行日志。".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 2);
    assert!(delivery[0].contains("**执行过程**"));
    assert!(delivery[0].contains("命令 `ls -t logs | head -2`"));
    assert!(delivery[0].contains("model_io.log"));
    assert!(delivery[0].contains("act_plan.log"));
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("这更像运行日志。")
    );
    assert!(crate::task_journal::delivery_payload_consistent(
        "这更像运行日志。",
        &delivery
    ));
    assert_eq!(
        final_answer_text_from_delivery(&delivery),
        "这更像运行日志。"
    );
}

#[test]
fn contract_matrix_delivery_suppresses_hardcoded_execution_summary() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list archive members".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "archive_basic".to_string(),
                args: serde_json::json!({"action": "list"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        "notes.txt\nnested/config.ini\n",
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveList;
    route.output_contract.requires_content_evidence = true;
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut delivery = vec![
        "**执行过程**\n1. 调用技能 `archive_basic`".to_string(),
        "notes.txt\nnested/config.ini".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["notes.txt\nnested/config.ini".to_string()]);
}

#[test]
fn evidence_contract_delivery_suppresses_execution_summary_for_name_list_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["README.txt"],"names_only":true}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut delivery = vec![
        "**Execution**\n1. Called tool `fs_basic`".to_string(),
        "README.txt".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["README.txt".to_string()]);
}

#[test]
fn evidence_contract_delivery_suppresses_execution_summary_for_status_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "http_basic",
        r#"status=200 {"ok":true}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut delivery = vec![
        "**Execution**\n1. Called skill `http_basic`".to_string(),
        "The health endpoint is reachable with HTTP 200.".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(
        delivery,
        vec!["The health endpoint is reachable with HTTP 200.".to_string()]
    );
}

#[test]
fn final_answer_text_from_delivery_joins_publishable_chunks() {
    let delivery = vec![
        "**执行过程**\n1. 调用技能 `read_file`".to_string(),
        "第一部分内容。".to_string(),
        "第二部分内容。".to_string(),
    ];

    assert_eq!(
        final_answer_text_from_delivery(&delivery),
        "第一部分内容。\n\n第二部分内容。"
    );
}

#[test]
fn execution_summary_uses_japanese_labels_for_japanese_request() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "logs ディレクトリのファイル名を3つだけ一覧して。".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "inventory_dir",
                    "path": "/tmp/logs",
                    "names_only": true,
                    "max_entries": 3
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        "act_plan.log\nclawd.log\nclawd.run.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        original_user_request: Some("logs ディレクトリのファイル名を3つだけ一覧して。".to_string()),
        ..Default::default()
    };
    let mut delivery = vec!["act_plan.log\nclawd.log\nclawd.run.log".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 2);
    assert!(delivery[0].contains("**実行過程**"));
    assert!(delivery[0].contains("スキル `system_basic`"));
    assert!(delivery[0].contains("出力："));
    assert!(crate::finalize::is_execution_summary_message(&delivery[0]));
}

#[test]
fn execution_summary_suppressed_for_scalar_value_contract() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "extract package name".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_field",
                    "path": "/tmp/package.json",
                    "field_path": "name"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"extract_field","field_path":"name","value_text":"rustclaw-nl-fixture"}"#,
    ));
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut delivery = vec!["rustclaw-nl-fixture".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["rustclaw-nl-fixture"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_drops_existing_summary_for_scalar_delivery_contract() {
    let route = scalar_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"description","value_text":"Local fixture package for RustClaw NL regression tests"}"#,
    ));
    let mut delivery = vec![
        "**実行過程**\n1. ツール `config_basic`を呼び出しました".to_string(),
        "Local fixture package for RustClaw NL regression tests".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(
        delivery,
        vec!["Local fixture package for RustClaw NL regression tests"]
    );
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_when_structured_keys_delivery_matches_scalar_observation(
) {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::StructuredKeys;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"structured_keys","path":"package.json","keys":["scripts.lint"]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts.lint","value":"echo lint","value_text":"echo lint","value_type":"string"}"#,
    ));
    let mut delivery = vec![
        "**Execution**\n1. Called tool `config_basic` with action `structured_keys`.".to_string(),
        "**Execution**\n2. Called tool `config_basic` with action `extract_field`.".to_string(),
        "echo lint".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["echo lint"]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_config_guard_delivery() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"guard_config","format":"toml","path":"/home/guagua/rustclaw/configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","risk_count":3,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true","telegram.sendfile.full_access=true"]}"#,
    ));
    let answer = "Found 3 config risk(s) in `/home/guagua/rustclaw/configs/config.toml`: tools.allow_sudo=true; tools.allow_path_outside_workspace=true; telegram.sendfile.full_access=true.";
    let mut delivery = vec![
        "**実行過程**\n1. スキル `config_edit`（action=guard_config）を呼び出しました".to_string(),
        answer.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![answer]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_transform_result_delivery() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "transform",
        r#"{"status":"ok","formatted":null,"result":[{"name":"beta"},{"name":"alpha"}]}"#,
    ));
    let answer = r#"[{"name":"beta"},{"name":"alpha"}]"#;
    let mut delivery = vec![
        "**Execution**\n1. Called skill `transform` (action=transform_data)".to_string(),
        answer.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![answer]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_strict_synthesized_delivery() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["README.md","configs"],"counts":{"total":2}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "README.md is documentation; configs contains settings.",
    ));
    let answer = "README.md is documentation; configs contains settings.";
    let mut delivery = vec![
        "**Execution**\n1. Called tool `fs_basic` (action=inventory_dir)".to_string(),
        answer.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![answer]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_synthesized_content_delivery() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
    ));
    let mut delivery = vec![
        "**执行过程**\n1. 调用工具 `config_basic`".to_string(),
        "该 scripts 字段定义了 build、dev、lint 三个脚本，均为 echo 占位命令。".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(
        delivery,
        vec!["该 scripts 字段定义了 build、dev、lint 三个脚本，均为 echo 占位命令。"]
    );
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_suppressed_for_multi_structured_scalar_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw-nl-fixture"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd"}"#,
    ));
    loop_state.last_publishable_synthesis_output =
        Some("rustclaw-nl-fixture and clawd are different.".to_string());
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_scalar_content_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r##"{"action":"read_text_range","path":"/tmp/release_checklist.md","content":"# Release Checklist\n\n1. Verify config."}"##,
    ));
    loop_state.last_publishable_synthesis_output = Some("Release Checklist".to_string());
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_attaches_each_execution_step_as_separate_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "tell joke and print pwd".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "pwd"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "date"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "Sun May 3\n"));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["为什么程序员喜欢黑夜？因为 bug 比较容易显现。".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 3);
    assert!(delivery[0].contains("命令 `pwd`"));
    assert!(delivery[0].contains("/home/guagua/rustclaw"));
    assert!(delivery[1].contains("命令 `date`"));
    assert!(delivery[1].contains("Sun May 3"));
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("为什么程序员喜欢黑夜？因为 bug 比较容易显现。")
    );
}

#[test]
fn execution_summary_uses_english_labels_for_english_requests() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list recent logs".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "ls -t logs | head -2"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "model_io.log\nact_plan.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let summary = build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("List the two most recently modified files in logs, then tell me what they are."),
    )
    .expect("execution summary");

    assert!(summary.starts_with("**Execution**"));
    assert!(summary.contains("1. Called command `ls -t logs | head -2`"));
    assert!(summary.contains("   Output:"));
    assert!(crate::finalize::is_execution_summary_message(&summary));
}

#[test]
fn execution_summary_does_not_reuse_same_step_id_from_wrong_round() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "pack archive".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "archive_basic".to_string(),
                    args: serde_json::json!({"action": "pack"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: "verify archive".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "system_basic".to_string(),
                    args: serde_json::json!({"action": "path_batch_facts"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "archive_basic", "exit=0\n"));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let summary = build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("Zip scripts/skill_calls into tmp/nl_archive_case_en.zip, then tell me briefly whether it succeeded."),
    )
    .expect("execution summary");

    assert!(summary.contains("Called skill `archive_basic`"));
    assert!(summary.contains("Called skill `system_basic`"));
    assert!(!summary.contains("Called skill `respond`"));
}

#[test]
fn execution_summary_uses_output_action_when_global_step_ids_shift() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read old config field".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "config_basic".to_string(),
                args: serde_json::json!({"action": "read_field"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: "edit config field".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "config_edit".to_string(),
                    args: serde_json::json!({"action": "plan_config_change"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "config_edit".to_string(),
                    args: serde_json::json!({"action": "apply_config_change"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_3".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "config_edit".to_string(),
                    args: serde_json::json!({"action": "validate_config"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_edit",
        r#"{"action":"plan_config_change"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "config_edit",
        r#"{"action":"apply_config_change"}"#,
    ));

    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let summary = build_execution_summary_message(&loop_state, Some(&ctx), Some("把配置项打开"))
        .expect("execution summary");

    assert!(summary.contains("action=plan_config_change"));
    assert!(summary.contains("action=apply_config_change"));
    assert!(!summary.contains("action=validate_config"));
}

#[test]
fn virtual_tool_execution_summary_uses_tool_label_without_plan_step() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","count":5}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let summary = build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("列出当前目录最近修改的文件"),
    )
    .expect("execution summary");

    assert!(summary.contains("调用工具 `fs_basic`"));
    assert!(!summary.contains("调用技能 `fs_basic`"));
}

#[test]
fn virtual_tool_execution_summary_uses_tool_label_even_when_plan_used_call_skill() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "compare file sizes".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "fs_basic".to_string(),
                args: serde_json::json!({"action": "stat_paths"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let summary =
        build_execution_summary_message(&loop_state, Some(&ctx), Some("Compare file sizes."))
            .expect("execution summary");

    assert!(summary.contains("Called tool `fs_basic`"));
    assert!(!summary.contains("Called skill `fs_basic`"));
}

#[tokio::test]
async fn observed_execution_without_delivery_reply_attaches_raw_summary() {
    let state = test_state();
    let task = claimed_task("task-missing-delivery-observed");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list recent logs".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "ls -t logs | head -2"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "model_io.log\nact_plan.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "列出 logs 最近两个文件，再判断类型",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(reply.should_fail_task);
    assert_eq!(reply.messages.len(), 2);
    assert!(reply.messages[0].contains("**执行过程**"));
    assert!(reply.messages[0].contains("命令 `ls -t logs | head -2`"));
    assert!(reply.messages[0].contains("model_io.log"));
    assert!(reply.messages[0].contains("act_plan.log"));
    assert!(!reply.text.contains("你最想看的是哪一项"));
}

#[test]
fn observed_synthesis_unavailable_fails_loud_and_keeps_execution_summary() {
    let state = test_state();
    let task = claimed_task("task-observed-llm-unavailable");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Cargo.toml\nREADME.md\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let reply = observed_synthesis_unavailable_reply(
        &state,
        &task,
        "列一下当前目录，然后总结一下",
        &loop_state,
        Some(&ctx),
        "No available LLM provider configured",
    );

    assert!(reply.should_fail_task);
    assert!(reply.text.contains("模型暂时不可用"));
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply.messages[0].contains("**执行过程**"));
    assert!(reply.messages[0].contains("Cargo.toml"));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Failure)
    );
}

#[tokio::test]
async fn observed_execution_without_delivery_skips_summary_for_extract_field_result() {
    let state = test_state();
    let task = claimed_task("task-missing-field-observed");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read package name".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_field",
                    "path": "package.json",
                    "field_path": "name"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"extract_field","exists":false,"field_path":"name","format":"json","path":"package.json","resolved_path":"/tmp/package.json","value":null,"value_text":"","value_type":"null"}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "读取 package.json 里的 name 字段，只输出值",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert_eq!(reply.messages.len(), 2);
    assert!(reply.messages[0].contains("**执行过程**"));
    assert!(reply.messages[0].contains("system_basic"));
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_structured_container_summary() {
    let state = test_state();
    let task = claimed_task("task-structured-container-summary");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts","format":"json","path":"package.json","resolved_field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "Read the scripts field from package.json and summarize it briefly.",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.text,
        "`scripts` contains 3 entries: build=echo build, dev=echo dev, lint=echo lint."
    );
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_matrix_grouped_name_answer() {
    let state = test_state();
    let task = claimed_task("task-matrix-grouped-no-delivery");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "workspace".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"files":["README.md"],"dirs":["configs","docs"],"other":[]},"path":"workspace"}"#,
    ));

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "list direct children grouped by kind",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "dirs:\n- configs\n- docs\nfiles:\n- README.md");
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.finalizer_summary.as_ref())
            .and_then(|summary| summary.format_ok),
        Some(true)
    );
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_matrix_hidden_entries_answer() {
    let state = test_state();
    let task = claimed_task("task-matrix-hidden-no-delivery");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":2,"hidden":2,"total":3},"entries":[{"hidden":true,"kind":"dir","name":".git","path":".git"},{"hidden":true,"kind":"file","name":".gitignore","path":".gitignore"},{"hidden":false,"kind":"file","name":"README.md","path":"README.md"}],"include_hidden":true,"names":[".git",".gitignore","README.md"],"path":"."}"#,
    ));

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "check hidden entries",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, ".git\n.gitignore");
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_docker_image_observation() {
    let state = test_state();
    let task = claimed_task("task-matrix-docker-images-no-delivery");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DockerImages;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "bash: line 1: docker: command not found\n",
    ));

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "list Docker images",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "bash: line 1: docker: command not found");
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn execution_summary_attaches_for_exact_observed_passthrough_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "print pwd".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "pwd"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["/home/guagua/rustclaw".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 2);
    assert!(delivery[0].contains("**执行过程**"));
    assert!(delivery[0].contains("命令 `pwd`"));
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("/home/guagua/rustclaw")
    );
}

#[test]
fn execution_summary_skips_for_raw_command_output_route() {
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_strict_content_excerpt_contract() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read tail".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "/tmp/model_io.log",
                    "mode": "tail",
                    "n": 10
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","excerpt":"1|alpha\n2|beta","path":"/tmp/model_io.log"}"#,
    ));

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_generic_path_content_contract() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.log".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|alpha\n2|beta","path":"logs/clawd.log"}"#,
    ));
    let mut delivery = vec![
        "**Execution**\n1. Called tool `fs_basic`".to_string(),
        "alpha\nbeta".to_string(),
    ];

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["alpha\nbeta".to_string()]);
}

#[test]
fn execution_summary_sanitizes_log_excerpt_secrets_and_ansi() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","excerpt":"1|\u001b[32mconnected\u001b[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef","path":"/tmp/feishud.log"}"#,
    ));

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_exact_file_names_contract() {
    let mut route = free_route_result();
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        "alpha.md\nbeta.md\n",
    ));
    let mut delivery = vec!["alpha.md\nbeta.md".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_exact_sentence_count_contract() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.exact_sentence_count = Some(3);
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "doc_parse",
        "RustClaw is a local Rust agent runtime centered on clawd.",
    ));
    let mut delivery = vec![
        "RustClaw 是一个本地 Rust agent 运行时。它以 clawd 为核心。它面向多渠道任务执行。"
            .to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert!(!crate::finalize::is_execution_summary_message(&delivery[0]));
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_scalar_count_contract() {
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","counts":{"total":64}}"#,
    ));
    let mut delivery = vec!["64".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["64"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_scalar_count_inventory_observation() {
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","counts":{"total":64}}"#,
    ));
    let mut delivery = vec!["64".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["64"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_strict_json_container_delivery() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["id","name"],"rows":[{"id":1,"name":"Alice"}]}"#,
    ));
    let mut delivery = vec![
        "**执行过程**\n1. 调用技能 `db_basic`".to_string(),
        r#"[{"id":1,"name":"Alice"}]"#.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![r#"[{"id":1,"name":"Alice"}]"#.to_string()]);
}

#[test]
fn execution_summary_suppressed_for_file_names_contract_even_with_original_user_request() {
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some("先列出 logs 目录下前 5 个文件名".to_string()),
        user_request: Some("List the first five filenames under logs.".to_string()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        "act_plan.log\nclawd.log\n",
    ));

    assert!(build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("List the first five filenames under logs."),
    )
    .is_none());
}

#[test]
fn execution_summary_attaches_for_failed_file_token_delivery() {
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "send file".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "read_file".to_string(),
                args: serde_json::json!({"path": "/tmp/missing.txt"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt",
    ));
    let mut delivery = vec!["File not found at the provided path.".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 2);
    assert!(delivery[0].contains("**执行过程**"));
    assert!(delivery[0].contains("read_file"));
    assert!(delivery[0].contains("file not found"));
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("File not found at the provided path.")
    );
}

#[test]
fn execution_summary_suppressed_for_successful_file_token_delivery() {
    let mut route = free_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "send file".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "path_batch_facts",
                    "path": "/tmp/report.txt"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"path":"/tmp/report.txt","fact":{"kind":"file","resolved_path":"/tmp/report.txt"}}]}"#,
    ));
    let mut delivery = vec![
        "**执行过程**\n1. 调用工具 `fs_basic`".to_string(),
        "FILE:/tmp/report.txt".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["FILE:/tmp/report.txt".to_string()]);
}

#[test]
fn execution_summary_suppressed_for_existence_with_path_contract() {
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_search",
        r#"{"action":"find_name","count":1,"results":["rustclaw.service"]}"#,
    ));
    let mut delivery = vec!["有，路径：rustclaw.service".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["有，路径：rustclaw.service"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_sqlite_table_names_contract() {
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableNamesOnly;
    route.output_contract.locator_hint = "/tmp/test.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list sqlite tables".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({
                    "command": "sqlite3 /tmp/test.sqlite \"SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;\""
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "orders\nusers\n"));
    let mut delivery = vec!["这个 SQLite 数据库里有表：orders、users。".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["这个 SQLite 数据库里有表：orders、users。"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_includes_direct_fs_search_structured_observation() {
    let route = free_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_search",
        r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#,
    ));
    let mut delivery = vec!["有，路径：rustclaw.service".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 2);
    assert!(crate::finalize::is_execution_summary_message(&delivery[0]));
    assert!(delivery[0].contains("fs_search"));
    assert!(delivery[0].contains("rustclaw.service"));
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("有，路径：rustclaw.service")
    );
}

#[test]
fn execution_summary_suppressed_for_scalar_contract_without_reading_user_text() {
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_hint = ".".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        ".git\n.gitignore\n",
    ));
    let mut delivery = vec!["有。示例：.git, .gitignore".to_string()];

    attach_execution_summary_to_delivery(
        &loop_state,
        Some(&ctx),
        Some("plain runtime text that is intentionally ignored"),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["有。示例：.git, .gitignore"]);
    assert!(build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("plain runtime text that is intentionally ignored"),
    )
    .is_none());
}

#[test]
fn exact_file_names_contract_prefers_observed_list_over_synthesized_sentence() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        "alpha.md\nbeta.md\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "document 目录下有 alpha.md 和 beta.md。",
    ));
    loop_state.last_publishable_synthesis_output =
        Some("document 目录下有 alpha.md 和 beta.md。".to_string());
    loop_state.last_user_visible_respond =
        Some("document 目录下有 alpha.md 和 beta.md。".to_string());
    let mut delivery = vec!["document 目录下有 alpha.md 和 beta.md。".to_string()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task_test",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    );

    assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
    assert!(finalizer_summary.is_some());
}

#[test]
fn active_bound_inventory_path_overrides_bare_path_directory_listing_contract() {
    let state = test_state();
    let task = claimed_task("task-active-bound-inventory-path");
    let mut route = free_route_result();
    route.resolved_intent = "List contents of directory scripts/nl_tests/fixtures/locator_smart/case_only\n\n### ACTIVE_EXECUTION_ANCHOR\nfollowup_source_request: find report\nfollowup_op_kind: Read\nfollowup_bound_target: case_only/report.md\nobserved_bound_target: case_only/report.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/locator_smart/case_only".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":1,"total":1},"entries":[{"kind":"file","name":"Report.MD","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","size_bytes":33}],"names":["Report.MD"],"names_by_kind":{"dirs":[],"files":["Report.MD"],"other":[]},"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only","resolved_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only"}"#,
    ));

    let (answer, _) = direct_path_from_active_bound_inventory(&loop_state, Some(&ctx))
        .expect("active bound target should select matching inventory entry path");
    assert_eq!(
        answer,
        "scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD"
    );

    let mut delivery = vec!["Report.MD".to_string()];
    let mut finalizer_summary = None;
    assert!(super::replace_delivery_with_matrix_observed_shape_answer(
        &state,
        &task,
        "scripts/nl_tests/fixtures/locator_smart/case_only",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    ));
    assert_eq!(
        delivery,
        vec!["scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD"]
    );
}

#[test]
fn matrix_shape_guard_replaces_unstructured_strict_list_with_observed_list() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-list");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":2,"ext":"md","results":["alpha.md","beta.md"],"root":"document"}"#,
    ));
    let mut delivery = vec!["document 目录下有 alpha.md 和 beta.md。".to_string()];
    let mut finalizer_summary = None;

    assert!(super::replace_delivery_with_matrix_observed_shape_answer(
        &state,
        &task,
        "列出 document 下的 md 文件名，只输出列表",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    ));

    assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("alpha.md\nbeta.md")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_strict_list_shape_builds_list_from_observed_json() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":2,"ext":"md","results":["document/beta.md","document/alpha.md"],"root":"document"}"#,
    ));

    let (answer, summary) =
        super::matrix_strict_list_observed_answer(&route, &loop_state).expect("matrix list answer");

    assert_eq!(answer, "alpha.md\nbeta.md");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_strict_list_shape_builds_hidden_entry_list_from_inventory() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":2,"hidden":2,"total":3},"entries":[{"hidden":true,"kind":"dir","name":".git","path":".git"},{"hidden":true,"kind":"file","name":".gitignore","path":".gitignore"},{"hidden":false,"kind":"file","name":"README.md","path":"README.md"}],"include_hidden":true,"names":[".git",".gitignore","README.md"],"path":"."}"#,
    ));

    let (answer, summary) = super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("matrix hidden entries answer");

    assert_eq!(answer, ".git\n.gitignore");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_grouped_name_list_shape_builds_groups_from_names_by_kind() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "workspace".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":5,"files":2,"total":7},"names_by_kind":{"files":["README.md","package.json"],"dirs":["configs","data","docs","logs","tmp"],"other":[]},"path":"workspace"}"#,
    ));

    let (answer, summary) = super::matrix_grouped_name_list_observed_answer(&route, &loop_state)
        .expect("matrix grouped name answer");

    assert_eq!(
        answer,
        "dirs:\n- configs\n- data\n- docs\n- logs\n- tmp\nfiles:\n- package.json\n- README.md"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn mixed_listing_contract_prefers_grounded_synthesis_after_file_read() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let answer = "这个仓库的 UI 更像一个独立前端，因为 UI/package.json 的 name 是 react-example，并且 UI 目录有独立构建脚本。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"names_by_kind":{"files":["Cargo.toml"],"dirs":["UI"],"other":[]},"path":"."}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"UI/package.json","excerpt":"1|{\n2|  \"name\": \"react-example\"\n3|}"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", answer));
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());

    let (actual, summary) =
        super::latest_grounded_synthesis_for_mixed_listing_contract(&route, &loop_state)
            .expect("mixed evidence synthesis");

    assert_eq!(actual, answer);
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn matrix_shape_guard_replaces_unstructured_grouped_name_list_with_observed_groups() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-grouped-name-list");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "workspace".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"files":["README.md"],"dirs":["configs","docs"],"other":[]},"path":"workspace"}"#,
    ));
    let mut delivery = vec!["workspace 下面有 configs、docs 和 README.md。".to_string()];
    let mut finalizer_summary = None;

    assert!(super::replace_delivery_with_matrix_observed_shape_answer(
        &state,
        &task,
        "list direct children grouped by kind",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    ));

    assert_eq!(
        delivery,
        vec!["dirs:\n- configs\n- docs\nfiles:\n- README.md"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("dirs:\n- configs\n- docs\nfiles:\n- README.md")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_shape_guard_replaces_unstructured_table_with_markdown_table() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-table");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "data/app.sqlite".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let mut delivery = vec!["数据库里有 orders 和 users 两张表。".to_string()];
    let mut finalizer_summary = None;

    assert!(super::replace_delivery_with_matrix_observed_shape_answer(
        &state,
        &task,
        "列出数据库表，输出表格",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    ));

    assert_eq!(delivery, vec!["| name |\n| --- |\n| orders |\n| users |"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("| name |\n| --- |\n| orders |\n| users |")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn exact_directory_names_contract_replaces_file_list_synthesis_with_parent_dirs() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryNames;
    route.resolved_intent = "Find directories containing .sh files".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":4,"ext":"sh","results":["build-all.sh","component_start/start-clawd.sh","scripts/check.sh","component_start/start-feishud.sh"],"root":""}"#,
    ));
    let file_list =
        "1. build-all.sh\n2. component_start/start-clawd.sh\n3. scripts/check.sh".to_string();
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &file_list,
    ));
    loop_state.last_user_visible_respond = Some(file_list.clone());
    loop_state.last_publishable_synthesis_output = Some(file_list.clone());
    let mut delivery = vec![file_list];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task_test",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    );

    assert_eq!(delivery, vec![".\ncomponent_start\nscripts"]);
    assert!(finalizer_summary.is_some());
}

#[test]
fn execution_summary_truncates_long_outputs_with_ascii_ellipsis() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let long_output = format!("{}END", "x".repeat(1000));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", &long_output));

    let summary =
        build_execution_summary_message(&loop_state, Some(&ctx), None).expect("execution summary");

    assert!(summary.contains("..."));
    assert!(!summary.contains("END"));
    assert!(
        summary.len() < 700,
        "summary should stay compact, got {} chars",
        summary.len()
    );
}

#[test]
fn preferred_route_clarify_question_only_uses_explicit_route_clarify() {
    let mut route = scalar_route_result();
    route.needs_clarify = true;
    route.clarify_question = "请确认要读取哪个文件？".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(
        super::preferred_route_clarify_question(Some(&ctx)),
        Some("请确认要读取哪个文件？")
    );

    let mut route = scalar_route_result();
    route.clarify_question = "不会被复用".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(super::preferred_route_clarify_question(Some(&ctx)), None);
}

#[test]
fn confirmation_resume_requires_enforce_mode() {
    let mut verify = verify_summary(crate::verifier::VerifyMode::ObserveOnly);
    assert!(!verify_summary_requires_resume_confirmation(&verify));

    verify.mode = crate::verifier::VerifyMode::Enforce;
    assert!(verify_summary_requires_resume_confirmation(&verify));

    verify.approved = false;
    assert!(!verify_summary_requires_resume_confirmation(&verify));
}

#[test]
fn content_evidence_routes_require_clarify_without_qualified_completion() {
    assert!(finalizer_requires_clarify(None, true, false));
    assert!(!finalizer_requires_clarify(None, true, true));

    let allow_fallback = finalizer_summary(crate::finalize::FinalizerDisposition::AllowFallback);
    assert!(finalizer_requires_clarify(
        Some(&allow_fallback),
        true,
        false
    ));
    assert!(!finalizer_requires_clarify(
        Some(&allow_fallback),
        true,
        true
    ));

    let qualified = finalizer_summary(crate::finalize::FinalizerDisposition::QualifiedCompletion);
    assert!(!finalizer_requires_clarify(Some(&qualified), true, false));
    assert!(!finalizer_requires_clarify(None, false, false));
}

#[test]
fn missing_publishable_delivery_can_finish_as_clarify() {
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        needs_clarify: Some(true),
        ..Default::default()
    };

    let (status, should_fail) =
        observed_execution_without_publishable_delivery_outcome(false, Some(&summary));
    assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Clarify);
    assert!(!should_fail);

    let (status, should_fail) =
        observed_execution_without_publishable_delivery_outcome(true, Some(&summary));
    assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Success);
    assert!(!should_fail);

    let (status, should_fail) =
        observed_execution_without_publishable_delivery_outcome(false, None);
    assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Failure);
    assert!(should_fail);
}

#[test]
fn successful_delivery_can_preserve_structured_user_input_clarify() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    assert_eq!(
        successful_delivery_final_status(&loop_state, None),
        crate::task_journal::TaskJournalFinalStatus::Success
    );

    loop_state.pending_user_input_required = true;
    assert_eq!(
        successful_delivery_final_status(&loop_state, None),
        crate::task_journal::TaskJournalFinalStatus::Clarify
    );
}

#[tokio::test]
async fn content_evidence_step_failure_answer_reports_real_error() {
    let state = test_state();
    let task = claimed_task("task-content-error-direct");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/etc/shadow".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "permission_denied",
                "error_text": "read_range failed for /etc/shadow",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "/etc/shadow"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "读 /etc/shadow 第一行",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("content evidence failure should be publishable");

    assert!(answer.contains("`/etc/shadow`"));
    assert!(answer.to_ascii_lowercase().contains("permission denied"));
    assert!(answer.contains("`clawd` 进程当前没有 sudo/root 权限"));
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn content_evidence_step_failure_answer_preserves_plan_path_without_locator_hint() {
    let state = test_state();
    let task = claimed_task("task-content-error-plan-target");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some("读 /etc/shadow 第一行".to_string()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read protected file".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "/etc/shadow",
                    "mode": "head",
                    "n": 1
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "fs_basic",
                "error_kind": "permission_denied",
                "error_text": "read operation failed: permission denied by the operating system",
                "platform": "linux"
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "Read the first line of /etc/shadow",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("content evidence failure should preserve structured plan target");

    assert!(answer.contains("`/etc/shadow`"));
    assert!(answer.contains("permission denied"));
    assert!(answer.contains("已尝试访问"));
    assert!(!answer.contains("`fs_basic` 步骤执行失败"));
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
}

#[tokio::test]
async fn content_evidence_recoverable_crypto_account_error_is_completion() {
    let state = test_state();
    let task = claimed_task("task-crypto-account-error");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let err = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "crypto", err));

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "查一下我现在的持仓。",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("recoverable crypto account error should be publishable");

    assert!(answer.contains("crypto account access failed on binance"));
    assert!(!answer.contains("__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__"));
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn content_evidence_db_query_error_is_completion() {
    let state = test_state();
    let task = claimed_task("task-db-query-error");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "query missing table".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "db_basic".to_string(),
                args: serde_json::json!({
                    "action": "sqlite_query",
                    "db_path": "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite",
                    "sql": "SELECT * FROM missing_table"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "db_basic",
        &format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "db_basic",
                "error_kind": "sqlite_query_failed",
                "error_text": "prepare query failed: no such table: missing_table",
                "platform": "linux"
            })
        ),
    ));

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "Read missing_table and explain the SQLite error.",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("db query error should be publishable");

    assert!(answer.contains("missing_table"));
    assert!(answer.contains("no such table"));
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn execution_summary_normalizes_recoverable_crypto_account_error() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_recoverable_failure_context = true;
    let err = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "crypto", err));

    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let summaries =
        build_execution_summary_messages(&loop_state, Some(&agent_run_context), Some("查一下持仓"));

    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].contains("crypto account access failed on binance"));
    assert!(!summaries[0].contains("__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__"));
}

#[test]
fn deterministic_observed_execution_status_answer_reports_mixed_results() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "health_check",
        r#"{"ok":true}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));

    let answer = deterministic_observed_execution_status_answer(
        &state,
        "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
        &loop_state,
    )
    .expect("mixed observed results should produce deterministic answer");

    assert!(answer.contains("第 1 步 `health_check` 成功"));
    assert!(answer.contains("第 2 步 `run_cmd` 失败"));
    assert!(answer.contains("exit code 127"));
}

#[test]
fn deterministic_missing_observed_target_answer_reports_missing_scalar_count_path() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config_copy".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"configs/config_copy"}],"include_missing":true}"#,
    ));

    let answer = deterministic_missing_observed_target_answer(
        &state,
        "查一下 configs/config_copy 下面有几个 toml 文件",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("missing path observation should produce a handled user answer");

    assert!(answer.contains("configs/config_copy"));
    assert!(answer.contains("不存在"));
    assert!(answer.contains("无法统计"));
}

#[test]
fn deterministic_missing_observed_target_answer_respects_scalar_existence_shape() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt","error":"not found"}],"include_missing":true}"#,
    ));

    let answer = deterministic_missing_observed_target_answer(
        &state,
        "检查 group_02 的 memo.txt 是否存在，只回答存在或不存在",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("missing existence observation should produce concise scalar answer");

    assert_eq!(answer, "不存在");
}

#[test]
fn deterministic_missing_observed_target_answer_defers_non_bilingual_template() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing-ja.txt".to_string();
    route.resolved_intent = "/tmp/rustclaw-missing-ja.txt が存在するか確認してください".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(
            "/tmp/rustclaw-missing-ja.txt が存在するか確認してください".to_string(),
        ),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"/tmp/rustclaw-missing-ja.txt","error":"not found"}],"include_missing":true}"#,
    ));

    assert!(deterministic_missing_observed_target_answer(
        &state,
        "/tmp/rustclaw-missing-ja.txt が存在するか確認してください",
        &loop_state,
        Some(&agent_run_context),
    )
    .is_none());
}

#[test]
fn deterministic_missing_observed_target_answer_reports_missing_archive_path() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveList;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/missing_bundle.zip".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let structured_error = serde_json::json!({
        "skill": "archive_basic",
        "error_kind": "not_found",
        "error_text": "archive not found: scripts/nl_tests/fixtures/device_local/tmp/missing_bundle.zip",
        "extra": {
            "path": "scripts/nl_tests/fixtures/device_local/tmp/missing_bundle.zip",
            "role": "archive"
        },
        "text": null
    });
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "archive_basic",
        &format!("__RC_SKILL_ERROR__:{structured_error}"),
    ));

    let answer = deterministic_missing_observed_target_answer(
        &state,
        "Try to list scripts/nl_tests/fixtures/device_local/tmp/missing_bundle.zip and report the failure clearly.",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("missing archive observation should produce a handled user answer");

    assert!(answer.contains("missing_bundle.zip"));
    assert!(answer.contains("could not find") || answer.contains("cannot be completed"));
}

#[test]
fn deterministic_missing_observed_target_answer_skips_after_later_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan/missing.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"plan/missing.md"}]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":1,"patterns":["agent_intelligence"],"results":["plan/agent_intelligence_architecture_plan_20260511.md"],"root":"plan"}"#,
    ));

    assert!(deterministic_missing_observed_target_answer(
        &state,
        "读取缺失文件；如果不存在，就搜索 fallback 文件。",
        &loop_state,
        Some(&agent_run_context),
    )
    .is_none());

    let (answer, _) =
        direct_scalar_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .expect("fallback success should become scalar answer");
    assert_eq!(
        answer,
        "plan/agent_intelligence_architecture_plan_20260511.md"
    );
}

#[test]
fn direct_structured_observed_answer_prefers_latest_path_result_for_exact_contract() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan".to_string();
    route.resolved_intent =
        "If the first plan path is missing, find execution_intent markdown files under plan"
            .to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"plan/missing.md"}]}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_search",
        r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));

    let (answer, summary) =
        direct_structured_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .expect("latest structured path result should answer exact path contract");

    assert!(answer.contains("plan/execution_intent_route_trace_cases.txt"));
    assert!(answer.contains("plan/execution_intent_routing_repair_plan_20260509.md"));
    assert!(!answer.contains("第 1 步"), "answer: {answer}");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn exact_path_observed_answer_replaces_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_ext","count":1,"ext":"md","patterns":["execution_intent.md"],"results":["plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 read_file 失败。第 2 步 fs_search 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-exact-path-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["plan/execution_intent_routing_repair_plan_20260509.md".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509.md")
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn path_locator_observed_answer_replaces_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-path-locator-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "plan/execution_intent_route_trace_cases.txt\nplan/execution_intent_routing_repair_plan_20260509.md"
                .to_string()
        ]
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn strict_existence_path_observed_answer_replaces_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":1,"patterns":["execution_intent.md"],"results":["plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-strict-existence-path-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["plan/execution_intent_routing_repair_plan_20260509.md".to_string()]
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn scalar_path_observed_answer_replaces_step_status_after_broad_fallback_search() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-scalar-path-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert!(
        delivery_messages[0].ends_with("plan/execution_intent_routing_repair_plan_20260509.md"),
        "answer: {}",
        delivery_messages[0]
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn scalar_observed_answer_replaces_run_cmd_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "platform": "linux",
            "extra": {
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "missing command",
                "output_truncated": false
            }
        })
    );
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "run_cmd", &err));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "/usr/bin/bash\n"));
    let status_summary = "第 1 步 `run_cmd` 失败。第 2 步 `run_cmd` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-scalar-run-cmd-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec!["/usr/bin/bash".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/usr/bin/bash")
    );
}

#[test]
fn loop_contract_scalar_observed_answer_replaces_status_but_keeps_progress() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    loop_state.output_contract = Some(contract);
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "run_cmd", "command failed"));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "/usr/bin/bash\n"));
    loop_state.delivery_messages.push(
        "**执行过程**\n1. 调用命令 `missing`\n   错误：\n```text\ncommand failed\n```".to_string(),
    );
    loop_state
        .delivery_messages
        .push("第 1 步 `run_cmd` 失败。第 2 步 `run_cmd` 成功。".to_string());
    let task = claimed_task("task-loop-contract-scalar");
    let mut finalizer_summary = None;

    assert!(super::replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 2);
    assert!(loop_state.delivery_messages[0].contains("执行过程"));
    assert_eq!(loop_state.delivery_messages[1], "/usr/bin/bash");
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/usr/bin/bash")
    );
}

#[test]
fn loop_contract_path_observed_answer_replaces_status_but_keeps_progress() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    loop_state.output_contract = Some(contract);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_ext","count":1,"results":["plan/execution_intent_routing_repair_plan_20260509.md"]}"#,
    ));
    loop_state.delivery_messages.push(
        "**执行过程**\n1. 调用技能 `read_file`\n   错误：\n```text\nfile not found\n```"
            .to_string(),
    );
    loop_state
        .delivery_messages
        .push("Step 1 `read_file` failed. Step 2 `fs_search` succeeded.".to_string());
    let task = claimed_task("task-loop-contract-path");
    let mut finalizer_summary = None;

    assert!(super::replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 2);
    assert!(loop_state.delivery_messages[0].contains("执行过程"));
    assert_eq!(
        loop_state.delivery_messages[1],
        "plan/execution_intent_routing_repair_plan_20260509.md"
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509.md")
    );
}

#[test]
fn loop_contract_observed_answer_preserves_explicit_json_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    loop_state.output_contract = Some(contract);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#,
    ));
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`".to_string());
    loop_state
        .delivery_messages
        .push(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string());
    let task = claimed_task("task-loop-contract-json");
    let mut finalizer_summary = None;

    assert!(!super::replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#)
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn grounded_terminal_respond_replaces_structured_json_delivery() {
    let task = claimed_task("task-grounded-terminal-respond");
    let mut route = scalar_route_result();
    route.resolved_intent = "extract current working directory".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let raw = r#"{"arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", raw));
    loop_state.delivery_messages.push(raw.to_string());
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_2".to_string(),
                action_type: "respond".to_string(),
                skill: "respond".to_string(),
                args: serde_json::json!({"content":"/home/guagua/rustclaw"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    let mut finalizer_summary = None;

    assert!(
        super::replace_structured_delivery_with_grounded_terminal_respond(
            &task,
            &mut loop_state,
            Some(&agent_run_context),
            &mut finalizer_summary,
        )
    );

    assert_eq!(
        loop_state.delivery_messages,
        vec!["/home/guagua/rustclaw".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/home/guagua/rustclaw")
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.grounded_ok),
        Some(true)
    );
}

#[test]
fn grounded_terminal_respond_rejects_ungrounded_content() {
    let task = claimed_task("task-grounded-terminal-respond-ungrounded");
    let mut route = scalar_route_result();
    route.resolved_intent = "extract current working directory".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let raw = r#"{"arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", raw));
    loop_state.delivery_messages.push(raw.to_string());
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_2".to_string(),
                action_type: "respond".to_string(),
                skill: "respond".to_string(),
                args: serde_json::json!({"content":"/tmp/not-observed"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    let mut finalizer_summary = None;

    assert!(
        !super::replace_structured_delivery_with_grounded_terminal_respond(
            &task,
            &mut loop_state,
            Some(&agent_run_context),
            &mut finalizer_summary,
        )
    );

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(raw)
    );
    assert!(loop_state.last_user_visible_respond.is_none());
    assert!(finalizer_summary.is_none());
}

#[test]
fn loop_contract_observed_answer_requires_contract_evidence_completeness() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.response_shape = crate::OutputResponseShape::Scalar;
    contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    loop_state.output_contract = Some(contract);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "a short answer\n"));
    loop_state
        .delivery_messages
        .push("Step 1 `run_cmd` succeeded.".to_string());
    let task = claimed_task("task-loop-contract-incomplete-evidence");
    let mut finalizer_summary = None;

    assert!(!super::replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("Step 1 `run_cmd` succeeded.")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn loop_contract_observed_answer_requires_matrix_strict_extractor_when_route_is_available() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    loop_state.output_contract = Some(route.output_contract.clone());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "unregistered_skill", "3\n"));
    loop_state
        .delivery_messages
        .push("Step 1 `unregistered_skill` succeeded.".to_string());
    let task = claimed_task("task-loop-contract-strict-extractor");
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut finalizer_summary = None;

    assert!(!super::replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("Step 1 `unregistered_skill` succeeded.")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn loop_contract_observed_answer_does_not_hide_later_failure() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    loop_state.output_contract = Some(contract);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "/tmp/value\n"));
    loop_state
        .executed_step_results
        .push(err_step_result("step_2", "run_cmd", "command failed"));
    loop_state
        .delivery_messages
        .push("Step 2 `run_cmd` failed.".to_string());
    let task = claimed_task("task-loop-contract-later-failure");
    let mut finalizer_summary = None;

    assert!(!super::replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));
    assert_eq!(loop_state.last_user_visible_respond, None);
}

#[test]
fn deterministic_observed_execution_status_answer_uses_structured_run_cmd_stderr() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 7",
            "platform": "linux",
            "extra": {
                "exit_code": 7,
                "stderr": "problem",
                "output_truncated": false
            }
        })
    );
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "READY\n"));
    loop_state
        .executed_step_results
        .push(err_step_result("step_2", "run_cmd", &err));

    let answer = deterministic_observed_execution_status_answer(
        &state,
        "执行两个命令，告诉我退出码和错误输出。",
        &loop_state,
    )
    .expect("mixed observed results should produce deterministic answer");

    assert!(answer.contains("exit code 7"), "answer: {answer}");
    assert!(answer.contains("stderr: problem"), "answer: {answer}");
}

#[test]
fn deterministic_observed_execution_status_answer_attaches_before_llm_fallback() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "health_check",
        r#"{"ok":true}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(attach_deterministic_observed_execution_status_answer(
        &state,
        &task,
        "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
        &mut loop_state,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(loop_state.delivery_messages[0].contains("第 1 步 `health_check` 成功"));
    assert!(loop_state.delivery_messages[0].contains("第 2 步 `run_cmd` 失败"));
    let summary = finalizer_summary.expect("summary");
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn observed_fallback_allowed_for_matrix_route_after_planned_synthesis() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["a.schema.json"],"entries":[{"name":"a.schema.json","size_bytes":1}]}"#,
    ));
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"synthesize_answer","evidence_refs":["last_output"]}]}"#,
    );

    assert!(should_try_observed_output_language_fallback(
        &loop_state,
        Some(&agent_run_context)
    ));
}

#[test]
fn content_answer_candidate_prevents_status_summary_replacement() {
    let mut route = free_route_result();
    route.resolved_intent = "List schema files and identify the largest one.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["intent_normalizer.schema.json"],"entries":[{"name":"intent_normalizer.schema.json","size_bytes":13124}]}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "system_basic",
        "action `system_basic.validate_structured` is rejected by contract `directory_entry_groups`",
    ));
    let delivery_messages =
        vec!["intent_normalizer.schema.json 最大；这个目录保存 JSON Schema。".to_string()];

    assert!(delivery_is_content_answer_candidate(
        Some(&agent_run_context),
        &loop_state,
        &delivery_messages
    ));
}

#[test]
fn deterministic_observed_execution_status_answer_replaces_bad_synthesis() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("步骤2未观察到执行结果，因此无法确认成功或失败。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "health_check",
        r#"{"ok":true}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(
        replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(loop_state.delivery_messages[0].contains("第 2 步 `run_cmd` 失败"));
    assert!(!loop_state.delivery_messages[0].contains("无法确认成功或失败"));
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.completion_ok),
        Some(true)
    );
}

#[test]
fn deterministic_observed_execution_status_keeps_recovered_content_answer() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-recovered");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let answer =
        "目标文件不存在；候选路径：plan/llm_first_agent_convergence_plan_20260511_已完成.md"
            .to_string();
    loop_state.delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"exists":false}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        r#"{"results":["plan/llm_first_agent_convergence_plan_20260511_已完成.md"]}"#,
    ));
    let mut finalizer_summary = None;

    assert!(
        !replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "读取缺失文件；如果不存在就返回候选路径",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );
    assert_eq!(loop_state.delivery_messages, vec![answer]);
    assert!(finalizer_summary.is_none());
}

#[test]
fn deterministic_observed_execution_status_keeps_planned_failed_step_answer() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-keep-planned");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "run two commands and report failed step".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "echo BEFORE_BREAK"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({
                        "command": "definitely_missing_command_rustclaw_user_ops_13579"
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_3".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({
                        "content": "第二步挂了，`definitely_missing_command_rustclaw_user_ops_13579` 命令不存在。"
                    }),
                    depends_on: vec!["step_2".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    let planned =
        "第二步挂了，`definitely_missing_command_rustclaw_user_ops_13579` 命令不存在。".to_string();
    loop_state.delivery_messages.push(planned.clone());
    loop_state.last_user_visible_respond = Some(planned.clone());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "BEFORE_BREAK\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_deterministic_observed_execution_status_answer(
        &state,
        &task,
        "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
        &mut loop_state,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages, vec![planned.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(planned.as_str())
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.completion_ok),
        Some(true)
    );
}

#[test]
fn deterministic_execution_failed_step_contract_replaces_verbose_status() {
    let state = test_state();
    let task = claimed_task("task-deterministic-failed-step-only");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "run two commands and identify only failed step".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "echo BEFORE_BREAK"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({
                        "command": "definitely_missing_command_rustclaw_user_ops_13579"
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.delivery_messages.push(
        "第 1 步 `run_cmd` 成功。第 2 步 `run_cmd` 失败：Command failed with exit code 127。"
            .to_string(),
    );
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "BEFORE_BREAK\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_deterministic_execution_failed_step_answer(
        &state,
        &task,
        "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
        &mut loop_state,
        Some(&ctx),
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 1);
    let answer = &loop_state.delivery_messages[0];
    assert!(answer.contains("第 2 步失败"), "answer: {answer}");
    assert!(answer.contains("definitely_missing_command_rustclaw_user_ops_13579"));
    assert!(!answer.contains("第 1 步"));
    assert!(!answer.contains("exit code 127"));
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.completion_ok),
        Some(true)
    );
}

#[test]
fn deterministic_observed_execution_status_replaces_raw_success_output() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-replace-raw");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("THINK_BREAK_CN".to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "THINK_BREAK_CN\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nbash: definitely_missing_command: command not found",
    ));
    let mut finalizer_summary = None;

    assert!(
        replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先执行第一个命令，再执行第二个命令，然后总结成功和失败分别是什么。",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(loop_state.delivery_messages[0].contains("第 1 步 `run_cmd` 成功"));
    assert!(loop_state.delivery_messages[0].contains("第 2 步 `run_cmd` 失败"));
    assert!(!loop_state.delivery_messages[0].trim().eq("THINK_BREAK_CN"));
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.completion_ok),
        Some(true)
    );
}

#[test]
fn exact_observed_answer_does_not_replace_mixed_failure_summary() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "BREAK_A\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let summary =
        "第 1 步 `run_cmd` 成功。第 2 步 `run_cmd` 失败：Command failed with exit code 127。"
            .to_string();
    let mut delivery_messages = vec![summary.clone()];
    let mut finalizer_summary = Some(super::deterministic_observed_execution_status_summary(
        &loop_state,
    ));

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-exact-observed-mixed-failure",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![summary]);
    assert_ne!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("BREAK_A")
    );
}

#[test]
fn raw_command_chatact_prefers_exact_observed_output_over_planned_extra_content() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/workspace/project\n",
    ));
    let planned = "/workspace/project\n\nworkspace ready".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned.clone()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-chatact-planned",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec!["/workspace/project".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/workspace/project")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_command_multiline_output_replaces_reordered_synthesis() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\n",
    ));
    let planned = "verify_task_termination.sh\nversion_info.sh\ntest_qwen_api.sh".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-reordered",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh".to_string()]
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_command_projection_plan_replaces_drifted_projected_answer() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "List /home/guagua/rustclaw/scripts in descending name order and return only five names."
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\nversion_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\n",
    ));
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"/home/guagua/rustclaw/scripts","names_only":true,"sort_by":"name_desc","max_entries":5}}]}"#,
    );
    let projected =
        "version_info.sh\nverify_task_termination.sh\nUntitled\ntest_qwen_api.sh\ntest_qwen_5_channels.py"
            .to_string();
    loop_state.last_user_visible_respond = Some(projected.clone());
    let mut delivery_messages = vec![projected.clone()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-structural-projection",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\ncheck-secrets.sh\nauth-key.sh"
                .to_string()
        ]
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_command_projection_plan_replaces_unprojected_listing_output() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "List /home/guagua/rustclaw/scripts in descending name order and return only five names."
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let raw_output = "Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\nversion_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\ntest_qwen_5_channels.py\ntest_minimax_curl.sh\n".to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", &raw_output));
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"/home/guagua/rustclaw/scripts","names_only":true,"sort_by":"name_desc","max_entries":5}}]}"#,
    );
    let mut delivery_messages = vec![raw_output];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-structural-projection-free",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\ntest_qwen_5_channels.py\ntest_minimax_curl.sh"
                .to_string()
        ]
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn backfill_suppresses_raw_run_cmd_when_plan_declares_projection() {
    let task = claimed_task("task-backfill-raw-projection");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\n",
    ));
    loop_state.last_user_visible_respond =
        Some("Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\n".to_string());
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"/home/guagua/rustclaw/scripts","sort_by":"name_desc","max_entries":5}}]}"#,
    );

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&agent_run_context));

    assert!(loop_state.delivery_messages.is_empty());
}

#[tokio::test]
async fn finalize_loop_reply_returns_graceful_result_for_permission_denied_content_evidence() {
    let state = test_state();
    let task = claimed_task("task-content-error-finalize");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/etc/shadow".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond =
        Some("我还没能根据现有证据生成可靠最终答案。".to_string());
    loop_state
        .delivery_messages
        .push("我还没能根据现有证据生成可靠最终答案。".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "permission_denied",
                "error_text": "read_range failed for /etc/shadow",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "/etc/shadow"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读 /etc/shadow 第一行",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a user-visible failure");

    assert!(reply.text.contains("`/etc/shadow`"));
    assert!(reply.text.contains("permission denied"));
    assert!(reply.text.contains("`clawd` 进程当前没有 sudo/root 权限"));
    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
}

#[tokio::test]
async fn finalize_loop_reply_does_not_infer_service_status_from_raw_systemd_text() {
    let state = test_state();
    let task = claimed_task("task-service-status-raw-systemd-text");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_hint.clear();
    route.output_contract.locator_hint = "telegramd.service".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(
            "Command failed with exit code 4\nstderr:\nUnit telegramd.service could not be found."
                .to_string(),
        ),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "check whether telegramd is running right now and briefly explain the status",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a user-visible command result");

    assert!(
        reply.should_fail_task,
        "raw systemd prose should not be promoted to a qualified service-status answer"
    );
    assert!(
        !reply.text.contains("no service unit"),
        "raw text should not trigger local service-status phrase inference: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_uses_structured_service_error_kind() {
    let state = test_state();
    let task = claimed_task("task-service-status-structured-missing");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let structured_error = serde_json::json!({
        "skill": "service_control",
        "error_kind": "not_found",
        "error_text": "no matching service found for the given target",
        "platform": "linux",
        "manager_type": "unknown",
        "service_name": "telegramd"
    });
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "service_control".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!("__RC_SKILL_ERROR__:{structured_error}")),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "check whether telegramd is running right now and briefly explain the status",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a service status answer");

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("telegramd"));
    assert!(reply.text.contains("not active"));
    assert!(reply.text.contains("no service unit"));
    assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
}

#[tokio::test]
async fn finalize_loop_reply_treats_structured_run_cmd_failure_as_user_result() {
    let state = test_state();
    let task = claimed_task("task-structured-run-cmd-nonzero");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let structured_error = serde_json::json!({
        "skill": "run_cmd",
        "error_kind": "nonzero_exit",
        "error_text": "Command failed with exit code 7",
        "platform": "linux",
        "extra": {
            "command": "printf problem >&2; exit 7",
            "exit_code": 7,
            "stderr": "problem",
            "output_truncated": false
        }
    });
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "run_cmd",
        &format!("__RC_SKILL_ERROR__:{structured_error}"),
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "执行命令 printf problem >&2; exit 7，告诉我退出码和错误输出。",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a user-visible command failure");

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("退出码为 7"), "text: {}", reply.text);
    assert!(
        reply.text.contains("错误输出为：problem"),
        "text: {}",
        reply.text
    );
    assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
}

#[tokio::test]
async fn finalize_loop_reply_sanitizes_contract_rejection_error() {
    let state = test_state();
    let task = claimed_task("task-contract-rejection-sanitized");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.locator_hint = "docs/release_checklist.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let structured_error = serde_json::json!({
        "skill": "system_basic",
        "error_kind": "contract_action_rejected",
        "error_text": "action `system_basic.inventory_dir` is rejected by contract `excerpt_kind_judgment`",
        "extra": {
            "action": "system_basic.inventory_dir",
            "contract_match": "excerpt_kind_judgment"
        }
    });
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "system_basic",
        &format!("__RC_SKILL_ERROR__:{structured_error}"),
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读取 release_checklist.md 开头并判断它像操作清单还是普通说明",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return sanitized failure text");

    assert!(reply.text.contains("planned tool step was not allowed"));
    assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
    assert!(!reply.text.contains("excerpt_kind_judgment"));
    assert!(!reply.text.contains("system_basic.inventory_dir"));
}

#[tokio::test]
async fn finalize_loop_reply_prefers_observed_raw_scalar_after_synthesis_error() {
    let state = test_state();
    let task = claimed_task("task-raw-scalar-synthesis-error");
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"runtime_status","kind":"current_user","value":"guagua","field_value":"guagua","command_output":"guagua"}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "synthesis failed",
    ));
    loop_state.delivery_messages.push(
        "获取到的当前用户名是 `guagua`。如果结果不符合预期，请提供更具体的查询条件。".to_string(),
    );
    loop_state.last_publishable_synthesis_output = loop_state.delivery_messages.last().cloned();

    let reply = finalize_loop_reply(
        &state,
        &task,
        "只输出当前用户名，不要解释",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should prefer observed scalar");

    assert_eq!(reply.text, "guagua");
    assert_eq!(reply.messages, vec!["guagua".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_treats_missing_read_target_as_user_result() {
    let state = test_state();
    let task = claimed_task("task-missing-read-target");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "document/missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path was not found: document/missing.txt",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "document/missing.txt"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读一下 document/missing.txt 开头，然后用一句话总结",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(!reply.should_fail_task);
    assert!(
        reply.text.contains("不存在")
            || reply.text.contains("未找到")
            || reply.text.to_ascii_lowercase().contains("not found")
            || reply.text.to_ascii_lowercase().contains("does not exist")
            || reply.text.to_ascii_lowercase().contains("no such file")
    );
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn content_evidence_missing_target_answer_uses_english_for_non_chinese_request() {
    let state = test_state();
    let task = claimed_task("task-missing-read-target-french");
    let answer = super::content_evidence_missing_target_answer(
        &state,
        &task,
        "Valide plan/does_not_exist_builtin_tool_case.toml comme TOML et explique l'echec clairement.",
        None,
        "__RC_READ_FILE_NOT_FOUND__:plan/does_not_exist_builtin_tool_case.toml",
    );

    assert!(answer.starts_with("I couldn't find"), "answer: {answer}");
    assert!(
        !answer.contains("未找到"),
        "non-Chinese missing-target fallback should not use Chinese: {answer}"
    );
}

#[test]
fn content_evidence_failure_suppresses_execution_summary_for_missing_target() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "system_basic",
        "__RC_READ_FILE_NOT_FOUND__:plan/does_not_exist_builtin_tool_case.toml",
    ));

    assert!(super::content_evidence_failure_suppresses_execution_summary(&loop_state));
}

#[tokio::test]
async fn missing_read_target_reply_prefers_original_user_language() {
    let state = test_state();
    let mut task = claimed_task("task-missing-read-target-language");
    task.payload_json = serde_json::json!({
        "text": "读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行"
    })
    .to_string();
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "./NO_SUCH_RUSTCLAW_TEST_987654.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path was not found: ./NO_SUCH_RUSTCLAW_TEST_987654.txt",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "./NO_SUCH_RUSTCLAW_TEST_987654.txt"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "Read the first line of the file ./NO_SUCH_RUSTCLAW_TEST_987654.txt.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(
        reply.text.contains("I couldn't find"),
        "text: {}",
        reply.text
    );
    assert!(!reply.text.contains("未找到"), "text: {}", reply.text);
}

#[tokio::test]
async fn missing_read_target_scalar_contract_keeps_failure_answer_not_path_only() {
    let state = test_state();
    let mut task = claimed_task("task-missing-read-target-scalar");
    task.payload_json = serde_json::json!({
        "text": "读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行"
    })
    .to_string();
    let mut route = scalar_route_result();
    route.resolved_intent =
        "用户请求读取文件 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行内容。".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "./NO_SUCH_RUSTCLAW_TEST_987654.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path was not found: ./NO_SUCH_RUSTCLAW_TEST_987654.txt",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "./NO_SUCH_RUSTCLAW_TEST_987654.txt"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "Read the first line of the file ./NO_SUCH_RUSTCLAW_TEST_987654.txt.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(
        reply.text.contains("I couldn't find"),
        "text: {}",
        reply.text
    );
    assert!(
        reply.text != "./NO_SUCH_RUSTCLAW_TEST_987654.txt",
        "missing target answer must not be reshaped into path-only scalar"
    );
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
}

#[tokio::test]
async fn finalize_loop_reply_treats_read_file_not_found_marker_as_user_result() {
    let state = test_state();
    let task = claimed_task("task-missing-read-target-marker");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "/tmp/missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt".to_string()),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读取 /tmp/missing.txt",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(!reply.should_fail_task);
    assert!(
        reply.text.contains("不存在")
            || reply.text.contains("未找到")
            || reply.text.to_ascii_lowercase().contains("not found")
            || reply.text.to_ascii_lowercase().contains("does not exist")
    );
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn execution_recipe_closeout_note_mentions_external_workspace_for_english_code_change() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        saw_external_target: true,
        ..Default::default()
    };

    let note = execution_recipe_closeout_note(
        None,
        "Fix the issue in /tmp/demo and verify it.",
        &loop_state,
    )
    .expect("closeout note");
    assert!(note.contains("external workspace"));
    assert!(note.contains("code changes"));
}

#[test]
fn execution_recipe_closeout_prefixes_greenfield_plain_text_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        saw_greenfield_creation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["Validation passed.".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "Create a new script and verify it works.",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].starts_with("Created the new artifact"));
    assert!(delivery[0].ends_with("Validation passed."));
}

#[test]
fn execution_recipe_closeout_does_not_infer_success_marker_from_user_text() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let mut delivery = vec!["修复已经完成。".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "修复系统服务并在通过时明确输出 VALIDATION_PASSED。",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].contains("系统范围"));
    assert!(!delivery[0].contains("VALIDATION_PASSED"));
    assert!(delivery[0].ends_with("修复已经完成。"));
}

#[test]
fn execution_recipe_closeout_prefixes_current_repo_plain_text_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["修复已经验证通过。".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "把当前仓库里的问题修好并验证。",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].starts_with("已在当前仓库完成代码修改"));
    assert!(delivery[0].ends_with("修复已经验证通过。"));
}

#[test]
fn execution_recipe_closeout_note_mentions_system_scope_for_english_ops() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };

    let note = execution_recipe_closeout_note(
        None,
        "Repair the system service and validate it.",
        &loop_state,
    )
    .expect("closeout note");
    assert!(note.contains("system scope"));
    assert!(note.contains("ops work"));
}

#[test]
fn execution_recipe_closeout_note_skips_apply_phase_without_validation() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };

    assert!(execution_recipe_closeout_note(
        None,
        "Repair the system service and validate it.",
        &loop_state,
    )
    .is_none());
}

#[test]
fn execution_recipe_closeout_skips_file_token_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        validation_required: true,
        saw_validation: true,
        saw_external_target: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["FILE:/tmp/report.txt".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "Update the config in another workspace and verify it.",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["FILE:/tmp/report.txt".to_string()]);
}

#[test]
fn execution_recipe_closeout_skips_scalar_route_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        validation_required: true,
        saw_validation: true,
        saw_external_target: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["42".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "Fix the value in /tmp/demo and just answer with the number.",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["42".to_string()]);
}

#[test]
fn execution_recipe_closeout_skips_scalar_route_when_marker_is_only_user_text() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let mut delivery = vec!["VALIDATION_PASSED".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "修复当前仓库问题，通过时明确输出 VALIDATION_PASSED。",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["VALIDATION_PASSED".to_string()]);
}

#[test]
fn ensure_requested_success_marker_visible_does_not_scan_user_text() {
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let mut delivery = vec!["Completed ops work at the system scope and validated it.".to_string()];

    ensure_requested_success_marker_visible(Some(&ctx), &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].contains("system scope"));
    assert!(!delivery[0].contains("VALIDATION_PASSED"));
}

#[test]
fn missing_requested_success_marker_does_not_scan_user_text() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["ops-repair-bad".to_string()];
    assert_eq!(
        missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}

#[test]
fn requested_success_marker_allows_recipe_success_when_present() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["VALIDATION_PASSED".to_string()];
    assert_eq!(
        missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}

#[test]
fn auto_requested_success_marker_stays_off_without_structured_request() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
    assert_eq!(
        auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}

#[test]
fn auto_requested_success_marker_stays_off_before_recipe_done() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
    assert_eq!(
        auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}

#[test]
fn direct_scalar_finalize_uses_structured_extract_field_missing_message() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar fallback should succeed");
    assert_eq!(answer, "未找到 name 字段");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_uses_structured_read_field_missing_message() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"read_field","exists":false,"field_path":"package.name","value_text":"","value":null,"value_type":"null"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar fallback should succeed");
    assert_eq!(answer, "未找到 package.name 字段");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_structured_observed_answer_skips_multi_evidence_content_routes() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_structured_observed_answer_skips_raw_passthrough_for_strict_exact_sentence() {
    let raw_snapshot = "exit=0\nState  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0      4096         0.0.0.0:8787      0.0.0.0:*    users:((\"clawd\",pid=117002,fd=31))";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "process_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw_snapshot.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_non_builtin_raw_answer_skips_synthesized_delivery_contract() {
    let raw_snapshot = "exit=0\nState  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0      4096         0.0.0.0:8787      0.0.0.0:*    users:((\"clawd\",pid=117002,fd=31))";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "process_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "process_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw_snapshot.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_non_builtin_skill_raw_answer(
        &test_state(),
        &loop_state,
        Some(&agent_run_context),
    )
    .is_none());
}

#[test]
fn direct_log_tail_status_answer_uses_log_levels_and_related_names() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"tree_summary","path":"logs","tree":{"children":[{"kind":"file","path":"logs/clawd.log"},{"kind":"file","path":"logs/clawd.run.log"},{"kind":"file","path":"logs/model_io.log"}]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":3,"path":"logs/clawd.run.log","excerpt":"101|2026-05-27T08:04:44Z INFO task_call started\n102|2026-05-27T08:04:45Z INFO task_call completed\n103|2026-05-27T08:04:46Z INFO finalize_ok"}"#,
    ));

    let (answer, summary) = direct_log_tail_status_answer(
        &state,
        "先列出 logs 目录里和 clawd 相关的文件名，再读 clawd.run.log 最后 20 行，最后只用一句中文说服务更像正常启动还是刚遇到报错",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("log tail status answer");

    assert!(answer.contains("clawd.log"));
    assert!(answer.contains("clawd.run.log"));
    assert!(answer.contains("log.level.info=3"));
    assert!(answer.contains("log.level.error=0"));
    assert!(answer.contains("log.state=ok"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn log_tail_status_replaces_unverified_synthesis() {
    let state = test_state();
    let task = claimed_task("task-log-tail-replace");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("无法验证日志内容是否支持结论。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"tree_summary","path":"logs","tree":{"children":[{"kind":"file","path":"logs/clawd.log"},{"kind":"file","path":"logs/clawd.run.log"}]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"path":"logs/clawd.run.log","excerpt":"1|2026-05-27T08:04:44Z INFO task_call started\n2|2026-05-27T08:04:45Z INFO finalize_ok"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_direct_log_tail_status_answer(
        &state,
        &task,
        "先列出 logs 目录里和 clawd 相关的文件名，再读 clawd.run.log 最后 20 行，最后只用一句中文说服务更像正常启动还是刚遇到报错",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    let answer = loop_state
        .delivery_messages
        .last()
        .expect("replacement answer");
    assert!(answer.contains("clawd.log"));
    assert!(answer.contains("clawd.run.log"));
    assert!(answer.contains("log.state=ok"));
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn log_tail_status_does_not_replace_content_excerpt_summary() {
    let state = test_state();
    let task = claimed_task("task-log-tail-summary-preserve");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let summary = "日志最后四条均为 ok 状态，表明最近调用已恢复为连续成功。";
    loop_state.delivery_messages.push(summary.to_string());
    loop_state.last_user_visible_respond = Some(summary.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":4,"path":"logs/model_io.log","excerpt":"7|{\"status\":\"ok\",\"response\":\"path resolved\"}\n8|{\"status\":\"ok\",\"response\":\"db inspected\"}\n9|{\"status\":\"ok\",\"response\":\"log tailed\"}\n10|{\"status\":\"ok\",\"response\":\"binding remembered\"}"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(direct_log_tail_status_answer(
        &state,
        "read the last four log lines and summarize the phenomenon",
        &loop_state,
        Some(&agent_run_context),
    )
    .is_none());
    assert!(!replace_delivery_with_direct_log_tail_status_answer(
        &state,
        &task,
        "read the last four log lines and summarize the phenomenon",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(loop_state.delivery_messages, vec![summary.to_string()]);
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_observed_answer_replaces_failed_synthesis_for_content_excerpt() {
    let state = test_state();
    let task = claimed_task("task-tail");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd_manual.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`（action=read_range）".to_string());
    loop_state
        .delivery_messages
        .push("由于日志输出被截断，无法查看最后2行内容。".to_string());
    loop_state.last_user_visible_respond =
        Some("由于日志输出被截断，无法查看最后2行内容。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"head","requested_n":40,"excerpt":"1|startup\n2|ready"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "由于日志输出被截断，无法查看最后2行内容。",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"4318|last alpha\n4319|last beta"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看最后一个最后 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("last alpha\nlast beta")
    );
    assert!(loop_state
        .delivery_messages
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("last alpha\nlast beta")
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn tail_read_range_observed_answer_allows_malformed_none_semantic_fs_basic() {
    let state = test_state();
    let task = claimed_task("task-tail-none");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .delivery_messages
        .push("已有执行结果，但我没能整理成可靠结论。".to_string());
    loop_state.last_user_visible_respond =
        Some("已有执行结果，但我没能整理成可靠结论。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1548|{\"task_id\":\"task-1\",\"omitted_fields\":[\"prompt\"]}\n1549|{\"task_id\":\"task-2\",\"omitted_fields\":[\"prompt\"]}"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看看最后 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    let answer = loop_state
        .last_user_visible_respond
        .as_deref()
        .unwrap_or("");
    assert!(answer.contains("task-1"));
    assert!(answer.contains("task-2"));
    assert!(!answer.contains("已有执行结果"));
}

#[test]
fn tail_read_range_replaces_machine_evidence_projection() {
    let state = test_state();
    let task = claimed_task("task-tail-machine-projection");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let machine_projection = "path=/home/guagua/rustclaw/logs/clawd.log\ncontent_excerpt:\n1|old";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .delivery_messages
        .push(machine_projection.to_string());
    loop_state.last_user_visible_respond = Some(machine_projection.to_string());
    loop_state.last_output = Some(machine_projection.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"10|fresh alpha\n11|fresh beta","path":"logs/clawd.log"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(looks_like_structured_machine_output(machine_projection));
    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看最近 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("fresh alpha\nfresh beta")
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("fresh alpha\nfresh beta")
    );
    assert!(finalizer_summary.is_some());
}

#[tokio::test]
async fn content_evidence_failure_defers_when_latest_tail_read_range_available() {
    let state = test_state();
    let task = claimed_task("task-tail-failure-defers");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "synthesize_answer",
        "synthesis failed",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|last alpha\n2|last beta"}"#,
    ));

    assert!(super::content_evidence_step_failure_reply_from_loop(
        &state,
        &task,
        "看看最后 2 行",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .is_none());
}

#[test]
fn tail_read_range_observed_answer_defers_one_sentence_summary() {
    let state = test_state();
    let task = claimed_task("task-tail-summary");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|a\n2|b"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "一句话总结最后两行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
}

#[test]
fn tail_read_range_observed_answer_preserves_existing_content_summary() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-summary");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let summary = "最后几行都是同一任务的工具调度记录。".to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`（action=read_range）".to_string());
    loop_state.delivery_messages.push(summary.clone());
    loop_state.last_user_visible_respond = Some(summary.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|raw alpha\n2|raw beta"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "查看最后两行，只做简短概述",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(summary.as_str())
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(summary.as_str())
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_observed_answer_replaces_older_summary_when_tail_synthesized_after_read() {
    let state = test_state();
    let task = claimed_task("task-tail-after-summary");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let older_summary = "model_io.log 里 error、failed、timeout 各出现 1 次。".to_string();
    let raw_tail_answer =
        "2026-05-20T09:00:00Z INFO prompt queued\n2026-05-20T09:00:01Z ERROR model timeout";
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `log_analyze`（action=summarize）".to_string());
    loop_state.delivery_messages.push(older_summary.clone());
    loop_state.last_user_visible_respond = Some(older_summary);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "log_analyze",
        r#"{"action":"summarize","counts":{"error":1,"failed":1,"timeout":1}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "model_io.log 里 error、failed、timeout 各出现 1 次。",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"31|2026-05-20T09:00:00Z INFO prompt queued\n32|2026-05-20T09:00:01Z ERROR model timeout"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "synthesize_answer",
        raw_tail_answer,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看下最近 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(raw_tail_answer)
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(raw_tail_answer)
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn tail_read_range_observed_answer_preserves_latest_registered_respond() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-respond");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let summary = "最后几行都是同一任务的工具调度记录。".to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`（action=read_range）".to_string());
    loop_state.delivery_messages.push(summary.clone());
    loop_state.last_user_visible_respond = Some(summary.clone());
    loop_state.last_output = Some(summary.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|raw alpha\n2|raw beta"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "查看最后两行，只做简短概述",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(summary.as_str())
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_observed_answer_preserves_synthesis_after_tail_for_raw_output() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-synthesis-after-tail");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let raw_tail_json = r#"{"action":"read_range","mode":"tail","requested_n":5,"excerpt":"7|{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"clarify\"}\n8|{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}\n9|{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}"}"#;
    let synthesis = "{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"clarify\"}\n{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}\n{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}\n\nAll records are ok and show one continuous model-handled task flow."
        .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(synthesis.clone());
    loop_state.last_user_visible_respond = Some(synthesis.clone());
    loop_state.last_publishable_synthesis_output = Some(synthesis.clone());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", raw_tail_json));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &synthesis,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "tail logs/model_io.log and provide the requested takeaway",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesis.as_str())
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(synthesis.as_str())
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn direct_structured_observed_answer_skips_ambiguous_multi_structured_scalars() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = false;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_structured_observed_answer_formats_scalar_equality_pair() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, summary) =
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("recent scalar equality should use structured field values");
    assert_eq!(answer, "react-example and clawd are different.");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_uses_hidden_entries_direct_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(".git\nREADME.md\n.env\nsrc\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.resolved_intent = "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = ".".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("hidden entries scalar fallback should succeed");
    assert_eq!(answer, "2");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn scalar_contract_prefers_latest_structured_observed_value_over_planned_delivery() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(
        "true (workspace inherited -- root workspace defines the actual version number)"
            .to_string(),
    );
    loop_state.last_user_visible_respond = loop_state.delivery_messages.last().cloned();
    loop_state.last_publishable_synthesis_output =
        Some("workspace.package.version: 0.1.7".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"package.version","format":"toml","resolved_field_path":"package.version","value":{"workspace":true},"value_text":"{\"workspace\":true}","value_type":"object"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"workspace.package.version","format":"toml","resolved_field_path":"workspace.package.version","value":"0.1.7","value_text":"0.1.7","value_type":"string"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "workspace.package.version: 0.1.7",
    ));
    let mut route = scalar_route_result();
    route.resolved_intent =
        "Read package.version from crates/clawd/Cargo.toml and output only the value.".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "crates/clawd/Cargo.toml".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(
            "Read package.version from crates/clawd/Cargo.toml and output only the value."
                .to_string(),
        ),
        ..Default::default()
    };
    let mut finalizer_summary = None;
    let mut delivery = vec![
        "true (workspace inherited -- root workspace defines the actual version number)"
            .to_string(),
    ];
    prefer_observed_answer_for_exact_contract(
        &state,
        "task-1",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery,
        &mut finalizer_summary,
    );

    assert_eq!(delivery, vec!["0.1.7".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("0.1.7")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn direct_scalar_finalize_defers_health_check_summary_to_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.resolved_intent =
        "执行基础健康检查，仅提取并返回操作系统相关的关键字段，排除 RustClaw 自身的状态摘要"
            .to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert!(
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
        "health_check scalar summary should be synthesized from observed evidence"
    );
}

#[test]
fn direct_scalar_finalize_reports_missing_path_before_extracting_path_field() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"configs/config_copy"}],"include_missing":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.resolved_intent = "查一下 configs/config_copy 下面有几个 toml 文件".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config_copy".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .expect("missing path should produce a scalar-compatible failure explanation");

    assert!(answer.contains("configs/config_copy"));
    assert!(answer.contains("不存在"));
    assert!(answer.contains("无法统计"));
    assert_ne!(answer.trim(), "configs/config_copy");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_does_not_repair_limited_listing_from_drifted_scalar_count() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"inventory_dir","path":"logs","resolved_path":"/tmp/logs","names_only":true,"sort_by":"mtime_desc","names":["clawd.run.log","model_io.log","act_plan.log"],"counts":{"total":3}}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.resolved_intent = "列出 logs 目录最近修改的 2 个文件名，只输出文件名".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar count fallback should follow the structured contract");
    assert_eq!(answer, "3");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn file_delivery_fallback_uses_ranked_inventory_after_placeholder_plan() {
    let dir = TempDirGuard::new("ranked_inventory_file_delivery");
    let newest = dir.path().join("newest.txt");
    let older = dir.path().join("older.txt");
    fs::write(&newest, "new").expect("write newest");
    fs::write(&older, "old").expect("write older");

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "deliver selected file from directory".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "list_dir",
                        "path": dir.path().display().to_string(),
                        "names_only": true,
                        "sort_by": "mtime_desc"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({
                        "content": format!("FILE:{}/{{{{last_output}}}}", dir.path().display())
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "inventory_dir",
            "resolved_path": dir.path().display().to_string(),
            "names_only": true,
            "sort_by": "mtime_desc",
            "names": ["newest.txt", "older.txt"],
            "counts": {"files": 2, "dirs": 0, "total": 2}
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = dir.path().display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (token, summary) = direct_file_token_from_observed_inventory(&loop_state, Some(&ctx))
        .expect("ranked inventory should recover file token");

    assert_eq!(token, format!("FILE:{}", newest.display()));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn file_delivery_fallback_uses_last_inventory_selection_from_placeholder_plan() {
    let dir = TempDirGuard::new("last_inventory_file_delivery");
    let first = dir.path().join("alpha.txt");
    let last = dir.path().join("zeta.txt");
    fs::write(&first, "first").expect("write first");
    fs::write(&last, "last").expect("write last");

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "deliver selected file from directory".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "list_dir",
                        "path": dir.path().display().to_string(),
                        "names_only": true,
                        "sort_by": "name"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({
                        "content": format!(
                            "FILE:{}/{{{{last_output.lines().last().unwrap()}}}}",
                            dir.path().display()
                        )
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "inventory_dir",
            "resolved_path": dir.path().display().to_string(),
            "names_only": true,
            "sort_by": "name",
            "names": ["alpha.txt", "zeta.txt"],
            "counts": {"files": 2, "dirs": 0, "total": 2}
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = dir.path().display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (token, summary) = direct_file_token_from_observed_inventory(&loop_state, Some(&ctx))
        .expect("explicit last selection over deterministic inventory should recover token");

    assert_eq!(token, format!("FILE:{}", last.display()));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn file_delivery_fallback_defers_ambiguous_unranked_inventory() {
    let dir = TempDirGuard::new("ambiguous_inventory_file_delivery");
    fs::write(dir.path().join("a.txt"), "a").expect("write a");
    fs::write(dir.path().join("b.txt"), "b").expect("write b");

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "inventory_dir",
            "resolved_path": dir.path().display().to_string(),
            "names_only": true,
            "sort_by": "name",
            "names": ["a.txt", "b.txt"],
            "counts": {"files": 2, "dirs": 0, "total": 2}
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_file_token_from_observed_inventory(&loop_state, Some(&ctx)).is_none());
}

#[test]
fn direct_scalar_finalize_preserves_planned_count_inventory_breakdown() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "count files and directories".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "count_inventory",
                    "path": ".",
                    "count_files": true,
                    "count_dirs": true
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"count_inventory","counts":{"total":66,"files":40,"dirs":26}}"#,
    ));
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some("帮我检查一下当前目录底下有多少个文件和文件夹。".to_string()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("planned component counts should be preserved");

    assert!(answer.contains("40"));
    assert!(answer.contains("26"));
    assert_ne!(answer.trim(), "66");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_uses_total_count_without_component_plan() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"count_inventory","counts":{"total":66,"files":40,"dirs":26}}"#,
    ));
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some("当前目录有多少个项目？只回复数字。".to_string()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("total count should be usable directly");

    assert_eq!(answer.trim(), "66");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_allows_scalar_count_with_one_sentence_shape() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","counts":{"total":34,"files":32,"dirs":2},"path":"document","recursive":false}"#,
    ));
    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some("再数一下 document 目录直接有多少个子项".to_string()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar count should not require scalar response shape");

    assert!(answer.contains("34"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_structured_finalize_answers_existence_with_path_from_single_observation() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.resolved_intent =
        "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, summary) =
        super::direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("single path_batch_facts observation should answer existence-with-path");
    assert_eq!(answer, "有，路径：/tmp/rustclaw-workspace/rustclaw.service");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_non_builtin_finalize_preserves_raw_skill_text() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "crypto".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "crypto".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .expect("non-builtin fallback should preserve raw text");
    assert_eq!(
        answer,
        "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_non_builtin_finalize_skips_structured_machine_output() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "stock".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "stock".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(r#"{"symbol":"AAPL","price":201.32}"#.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none()
    );
}

#[test]
fn backfill_delivery_prefers_contractual_last_respond_over_synthesis() {
    let task = claimed_task("task-contractual-last-respond");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("/home/guagua/rustclaw".to_string());
    loop_state.last_publishable_synthesis_output =
        Some("命令执行已完成，但综合答案时出错。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_hint.clear();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert_eq!(
        loop_state.delivery_messages,
        vec!["/home/guagua/rustclaw".to_string()]
    );
}

#[tokio::test]
async fn finalize_loop_reply_keeps_exact_single_line_observed_respond() {
    let state = test_state();
    let task = claimed_task("task-single-line-observed-respond");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("/home/guagua/rustclaw".to_string());
    loop_state.last_publishable_synthesis_output =
        Some("执行成功了，但合成最终答案的环节遇到问题。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "synthesis failed",
    ));
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "执行命令 pwd，直接回复执行结果，不要解释",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, "/home/guagua/rustclaw");
    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some("/home/guagua/rustclaw")
    );
    assert!(reply.messages[0].contains("**执行过程**"));
    assert!(reply.messages[0].contains("run_cmd"));
}

#[tokio::test]
async fn finalize_loop_reply_uses_publishable_synthesis_output() {
    let state = test_state();
    let task = claimed_task("task-synth-finalize");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("rustclaw.service".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("有，路径：/tmp/rustclaw.service".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output =
        Some("有，路径：/tmp/rustclaw.service".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "检查 rustclaw.service 是否存在并给出路径",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, "有，路径：/tmp/rustclaw.service");
    assert_eq!(reply.messages, vec!["有，路径：/tmp/rustclaw.service"]);
    assert!(!reply.should_fail_task);
    assert!(!reply.is_llm_reply);
}

#[tokio::test]
async fn finalize_loop_reply_replaces_raw_read_delivery_with_latest_synthesis() {
    let state = test_state();
    let task = claimed_task("task-raw-read-delivery-synthesis");
    let raw_read = r#"{"action":"read_range","mode":"head","excerpt":"1|alpha\n2|beta\n3|gamma","path":"/tmp/app.log"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", raw_read));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "검색 결과 없음",
    ));
    loop_state.delivery_messages.push(raw_read.to_string());
    loop_state.last_user_visible_respond = Some(raw_read.to_string());
    loop_state.last_publishable_synthesis_output = Some("검색 결과 없음".to_string());
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "app.log 에서 impossible_keyword_987 을 찾아보고 결과를 짧게 말해.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should use synthesis");

    assert_eq!(reply.text, "검색 결과 없음");
    assert_eq!(reply.messages, vec!["검색 결과 없음".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_uses_latest_fs_basic_path_fact_after_repair() {
    let state = test_state();
    let task = claimed_task("task-path-fact-after-repair");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":4,"facts":[{"exists":false,"path":"agent_guard.toml"},{"exists":false,"path":"audio.toml"},{"exists":false,"path":"browser_web_wait_map.json"},{"exists":false,"path":"channel_commands.toml"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"configs/channels","resolved_path":"/tmp/repo/configs/channels","size_bytes":4096},"path":"/tmp/repo/configs/channels"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "查看 configs 目录下最后一个条目的路径和类型信息".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/repo/configs/channels".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "看最后一个的基本信息，只回答路径和类型",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, "/tmp/repo/configs/channels | 目录");
    assert!(!reply.text.contains("没能整理成可靠结论"));
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some("/tmp/repo/configs/channels | 目录")
    );
    assert!(!reply.should_fail_task);
    assert!(!reply.is_llm_reply);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_synthesis_over_raw_last_respond() {
    let state = test_state();
    let task = claimed_task("task-synth-over-raw");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    let raw_git = "exit=0\nabc123 fix deployment docs\n";
    loop_state.last_user_visible_respond = Some(raw_git.to_string());
    loop_state.last_publishable_synthesis_output =
        Some("RustClaw 的部署可按项目文档和安装脚本完成。".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw_git.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("RustClaw 的部署可按项目文档和安装脚本完成。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "帮我写一段 RustClaw 部署说明",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, "RustClaw 的部署可按项目文档和安装脚本完成。");
    assert!(reply.messages[0].contains("**执行过程**"));
    assert!(reply.messages[0].contains("git_basic"));
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some("RustClaw 的部署可按项目文档和安装脚本完成。")
    );
}

#[tokio::test]
async fn finalize_loop_reply_keeps_article_synthesis_after_repair_success() {
    let state = test_state();
    let task = claimed_task("task-synth-after-repair");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("file operation failed: target path was not found".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "read_file".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("# RustClaw\n\nRustClaw is a local Rust agent runtime.".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let article = "RustClaw 是一个本地优先的 Rust 智能体运行时，围绕 clawd、技能调度和多渠道入口组织，可用于通过聊天或浏览器完成项目管理与自动化任务。".to_string();
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(article.clone()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.delivery_messages.push(
        "**执行过程**\n1. 调用技能 `list_dir`\n   错误：\n```text\nfile operation failed: target path was not found\n```"
            .to_string(),
    );
    loop_state.delivery_messages.push(article.clone());
    loop_state.last_user_visible_respond = Some(article.clone());
    loop_state.last_publishable_synthesis_output = Some(article.clone());
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "帮我写一篇关于 RustClaw 的长文",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, article);
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some(article.as_str())
    );
    assert!(
        !reply.text.contains("第 1 步"),
        "article synthesis must not be replaced by step status: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_replaces_template_placeholder_with_synthesis() {
    let state = test_state();
    let task = claimed_task("task-synth-placeholder");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("{{synthesized}}".to_string());
    loop_state.last_user_visible_respond = Some("{{synthesized}}".to_string());
    loop_state.last_publishable_synthesis_output =
        Some("RustClaw 可以按 README 中的安装脚本路径完成部署。".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "read_file".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("# RustClaw\n\nUse install-rustclaw-cmd.sh".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("RustClaw 可以按 README 中的安装脚本路径完成部署。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "帮我写一段 RustClaw 部署说明",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(
        reply.text,
        "RustClaw 可以按 README 中的安装脚本路径完成部署。"
    );
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some("RustClaw 可以按 README 中的安装脚本路径完成部署。")
    );
    assert!(!reply.text.contains("{{"));
}

#[test]
fn strict_scalar_count_keeps_planned_explanatory_answer() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "55\n"));
    loop_state.last_user_visible_respond =
        Some("55 个。当前范围内共有这么多普通文件。".to_string());
    let mut delivery_messages = vec!["55 个。当前范围内共有这么多普通文件。".to_string()];
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.exact_sentence_count = Some(1);
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-strict-scalar-count",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["55 个。当前范围内共有这么多普通文件。"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("55 个。当前范围内共有这么多普通文件。")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn exact_contract_keeps_publishable_synthesis_over_raw_observed_inventory() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"ext_filter":["md"],"names":["regression_llm_first","垃圾代码端分析报告.md"],"names_only":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("垃圾代码端分析报告.md".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_user_visible_respond = Some("垃圾代码端分析报告.md".to_string());
    loop_state.last_publishable_synthesis_output = Some("垃圾代码端分析报告.md".to_string());
    let mut delivery_messages = vec!["垃圾代码端分析报告.md".to_string()];
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_hint = "document".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-synth-file-names",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec!["垃圾代码端分析报告.md"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("垃圾代码端分析报告.md")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn exact_contract_keeps_model_language_verdict_over_observed_scalar() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"kind":"missing","path":"/tmp/rustclaw-missing-ja.txt"}],"include_missing":true}"#,
    ));
    let planned = "ファイルは存在しません。".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned.clone()];
    let mut route = scalar_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
        "Check if /tmp/rustclaw-missing-ja.txt exists; if not, respond briefly in Japanese"
            .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing-ja.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-ja-existence-verdict",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![planned.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(planned.as_str())
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn exact_contract_keeps_planned_subset_over_raw_observed_file_paths() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":4,"ext":"toml","results":["Cargo.toml","configs/config.toml","configs/skills_registry.toml","crates/clawd/Cargo.toml"]}"#,
    ));
    let planned = "Cargo.toml\nconfigs/config.toml\nconfigs/skills_registry.toml".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned.clone()];
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-planned-subset-file-paths",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![planned]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("Cargo.toml\nconfigs/config.toml\nconfigs/skills_registry.toml")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn exact_contract_keeps_explicit_json_delivery_over_observed_phrase() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/home/guagua/rustclaw/README.md","size_bytes":24929},"path":"/home/guagua/rustclaw/README.md"}],"fields":["exists","size"],"include_missing":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_user_visible_respond =
        Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string());
    let mut delivery_messages =
        vec![r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string()];
    let mut route = scalar_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "README.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-strict-json-delivery",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#)
    );
    assert!(finalizer_summary.is_none());
}

#[tokio::test]
async fn direct_publishable_observed_answer_skips_run_cmd_without_explicit_raw_contract() {
    let state = test_state();
    let task = claimed_task("task-no-raw-run-cmd-passthrough");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("/home/guagua/rustclaw\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_publishable_observed_answer(
        &state,
        &task,
        &loop_state,
        Some(&agent_run_context)
    )
    .await
    .is_none());
}

#[tokio::test]
async fn direct_publishable_observed_answer_skips_strict_run_cmd_format_contract() {
    let state = test_state();
    let task = claimed_task("task-strict-run-cmd-format");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("/home/guagua/rustclaw\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_publishable_observed_answer(
        &state,
        &task,
        &loop_state,
        Some(&agent_run_context)
    )
    .await
    .is_none());
}

#[test]
fn observed_output_language_fallback_skips_matrix_deterministic_shape() {
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(!agent_context_allows_observed_output_language_fallback(
        Some(&agent_run_context)
    ));
    assert!(agent_context_allows_observed_output_language_fallback(None));
}

#[tokio::test]
async fn direct_publishable_observed_answer_skips_matrix_deterministic_shape() {
    let state = test_state();
    let task = claimed_task("task-matrix-strict-no-raw-publishable");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("README.md\nCargo.toml\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_publishable_observed_answer(
        &state,
        &task,
        &loop_state,
        Some(&agent_run_context)
    )
    .await
    .is_none());
}

#[test]
fn direct_scalar_finalize_accepts_strict_single_line_observation() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("ThinkPad-X1\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("direct scalar answer");
    assert_eq!(answer, "ThinkPad-X1");
    assert!(summary.contract_ok);
}

#[test]
fn direct_scalar_finalize_skips_strict_raw_command_output_contract() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("ThinkPad-X1\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none());
}

#[test]
fn raw_structured_passthrough_is_dropped_for_scalar_contract() {
    let raw = r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(raw.to_string());
    loop_state.delivery_messages.push(raw.to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            raw
        ),
        Some(true)
    );
}

#[test]
fn structured_user_input_delivery_is_not_dropped_as_raw_passthrough() {
    let message = "Please provide the source directory.";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.pending_user_input_required = true;
    loop_state.last_user_visible_respond = Some(message.to_string());
    loop_state.delivery_messages.push(message.to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "photo_organize".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(message.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            message
        ),
        None
    );
}

#[test]
fn qualified_scalar_passthrough_is_not_dropped() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("rustclaw".to_string());
    loop_state.delivery_messages.push("rustclaw".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("rustclaw\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            "rustclaw"
        ),
        Some(false)
    );
}

#[test]
fn scalar_path_from_write_file_is_not_dropped_as_meta_placeholder() {
    let path = "/home/guagua/rustclaw/document/pwd_line.txt";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(path.to_string());
    loop_state.delivery_messages.push(path.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "write_file",
        "written 48 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
    ));
    loop_state
        .output_vars
        .insert("last_file_path".to_string(), path.to_string());
    loop_state
        .written_file_aliases
        .insert("pwd_line.txt".to_string(), path.to_string());
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_hint = "pwd_line.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            path
        ),
        Some(false)
    );
}

#[test]
fn content_evidence_contractual_terminal_answer_is_kept_before_meta_classifier() {
    let answer = "最先该做的是：验证配置能否正确加载。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", answer));
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "release_checklist.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            answer,
        ),
        Some(false)
    );
}

#[test]
fn content_evidence_one_sentence_terminal_answer_is_kept_without_semantic_kind() {
    let answer = "最先该做的是**验证配置能正确加载**。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", answer));
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
}

#[test]
fn content_evidence_contractual_terminal_answer_requires_observation() {
    let answer = "配置加载检查应先做。";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", answer));
    let mut route = free_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(!content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
}

#[test]
fn raw_listing_passthrough_is_dropped_for_content_evidence_free_shape() {
    let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(listing.to_string());
    loop_state.delivery_messages.push(listing.to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(format!("{listing}\n")),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            listing
        ),
        Some(true)
    );
}

#[test]
fn single_listing_entry_passthrough_is_dropped_for_content_evidence() {
    let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("base_skill_response_contract.md".to_string());
    loop_state
        .delivery_messages
        .push("base_skill_response_contract.md".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(format!("{listing}\n")),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::DirectoryPurposeSummary,
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some("/tmp/docs".to_string()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            "base_skill_response_contract.md"
        ),
        Some(true)
    );
}

#[test]
fn direct_scalar_finalize_prefers_presence_plus_path_for_fs_search_presence_queries() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_search".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.resolved_intent =
        "检查仓库工作区中是否存在 rustclaw.service 文件，如果存在则返回路径，如果不存在则返回不存在。回答格式只输出有或没有以及路径。"
            .to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("presence+path fallback should succeed");
    assert_eq!(answer, "有，路径：rustclaw.service");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn archive_exit_zero_passthrough_is_dropped_when_structured_answer_exists() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("exit=0".to_string());
    loop_state.delivery_messages.push("exit=0".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "archive_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    discard_raw_passthrough_delivery_when_structured_answer_available(
        &claimed_task("task-archive"),
        &mut loop_state,
        Some(&agent_run_context),
    );

    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.last_user_visible_respond.is_none());
}

#[test]
fn raw_publishable_guard_rejects_structured_json_payloads() {
    assert!(looks_like_structured_machine_output(
        r#"{"hostname":"rustclaw-test-host.local","cwd":"/tmp/rustclaw-workspace"}"#
    ));
    assert!(looks_like_structured_machine_output(
        r#"[{"name":"README.md"},{"name":"Cargo.toml"}]"#
    ));
    assert!(!looks_like_structured_machine_output(
        "rustclaw-test-host.local"
    ));
    assert!(!looks_like_structured_machine_output(
        "package_manager=brew"
    ));
    assert!(looks_like_structured_machine_output(
        "count[0].target=docs\ncount[0].total=3\ncount[1].target=logs\ncount[1].total=2"
    ));
    assert!(looks_like_structured_machine_output(
        "git.branch=main\ngit.clean=true"
    ));
}

#[test]
fn raw_publishable_guard_rejects_multi_line_command_snapshots() {
    assert!(looks_like_raw_command_snapshot(
        "exit=0\nCOMMAND PID USER\nclawd 4498 testuser TCP *:8787 (LISTEN)\n"
    ));
    assert!(!looks_like_raw_command_snapshot("testuser"));
}

#[test]
fn package_manager_summary_uses_structured_detect_answer() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "package_manager".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "package_manager".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("package_manager=brew".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "check which package manager is recognized and briefly say the everyday default"
            .to_string();
    route.route_reason = "llm_contract:package_manager_detect_summary".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let structured_answer =
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context));
    assert_eq!(
        structured_answer
            .as_ref()
            .map(|(answer, _summary)| answer.as_str()),
        Some(
            "Detected package manager: brew. Basis: package_manager returned package_manager=brew."
        ),
        "package manager summary should use structured skill evidence"
    );

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none(),
        "one-sentence summary should not raw-passthrough package_manager output"
    );
}

#[test]
fn git_status_summary_defers_to_synthesis_instead_of_raw_passthrough() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent = "检查当前仓库是否有未提交改动，用一句话告诉我".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
        "git status summary should be synthesized from observed evidence"
    );

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none(),
        "one-sentence summary should not raw-passthrough git status output"
    );
}

#[test]
fn git_repository_state_direct_answer_overrides_later_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("该仓库有 8 个文件存在未提交改动。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "检查当前仓库是否有未提交改动，用一句话告诉我".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GitRepositoryState;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, _summary) =
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("git repository state direct answer");
    assert_eq!(answer, "git.branch=main git.worktree=dirty");
}

#[test]
fn scalar_git_log_does_not_use_non_builtin_raw_passthrough() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\n09342a6a fix: expose nl execution and locator flows\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = scalar_route_result();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none(),
        "scalar git requests should use structured extraction or synthesis, not raw passthrough"
    );
}

#[test]
fn file_token_auto_locator_wraps_bare_filename_under_directory() {
    let temp = TempDirGuard::new("file_token_dir");
    let file_path = temp.path().join("report.txt");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    assert_eq!(
        resolve_file_token_from_auto_locator_answer(
            "report.txt",
            Some(temp.path().to_string_lossy().as_ref())
        )
        .as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn file_token_auto_locator_normalizes_delivery_messages() {
    let temp = TempDirGuard::new("file_token_messages");
    let file_path = temp.path().join("report.txt");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.last_user_visible_respond = Some("report.txt".to_string());
    loop_state.delivery_messages.push("report.txt".to_string());

    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
        ..Default::default()
    };

    normalize_file_token_delivery_from_auto_locator(&mut loop_state, Some(&agent_run_context));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(loop_state.delivery_messages, vec![expected]);
}

#[test]
fn file_token_auto_locator_recovers_from_observed_bare_filename() {
    let temp = TempDirGuard::new("file_token_observed_bare_filename");
    let file_path = temp.path().join("report.txt");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "report.txt\n"));

    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
        ..Default::default()
    };

    let (token, summary) = direct_file_token_from_observed_auto_locator_filename(
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("bare filename observation under auto locator should recover file token");

    assert_eq!(token, expected);
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn file_token_observed_path_normalizes_bare_filename_delivery() {
    let temp = TempDirGuard::new("file_token_observed_path");
    let file_path = temp.path().join("document/report.txt");
    fs::create_dir_all(file_path.parent().expect("parent")).expect("mkdir");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    let mut state = test_state();
    state.skill_rt.workspace_root = temp.path().to_path_buf();
    state.skill_rt.default_locator_search_dir = temp.path().to_path_buf();

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.last_user_visible_respond = Some("FILE:report.txt".to_string());
    loop_state
        .delivery_messages
        .push("FILE:report.txt".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "entries": [
                    {"name": "report.txt", "path": "document/report.txt"}
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    normalize_file_token_delivery_from_observed_paths(
        &state,
        &mut loop_state,
        Some(&agent_run_context),
    );

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(loop_state.delivery_messages, vec![expected]);
}

#[test]
fn missing_file_search_evidence_detects_zero_match_fs_search() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_search".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "find_name",
                "count": 0,
                "results": [],
                "root": ""
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn missing_file_search_evidence_detects_missing_path_facts() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn latest_file_delivery_observation_treats_missing_path_facts_as_terminal_missing() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output =
        Some("文件 /tmp/definitely-missing.txt 不存在，无法发送。".to_string());
    loop_state.last_user_visible_respond = loop_state.last_publishable_synthesis_output.clone();
    loop_state.delivery_messages = vec![loop_state
        .last_publishable_synthesis_output
        .clone()
        .unwrap()];

    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(latest_file_delivery_observation_is_missing(&loop_state));
    assert!(should_return_missing_file_delivery_reply(
        &loop_state,
        Some(&agent_run_context)
    ));
}

#[test]
fn missing_file_search_evidence_detects_not_found_probe_output() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("NOT_FOUND\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn missing_file_search_evidence_detects_system_basic_find_path_zero_matches() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "find_path",
                "count": 0,
                "matches": [],
                "query": "missing.md",
                "target_kind": "file"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[tokio::test]
async fn finalize_loop_reply_returns_not_found_for_missing_file_delivery() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "definitely_missing_named_file.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_search".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "find_name",
                "count": 0,
                "results": [],
                "root": ""
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 definitely_missing_named_file.txt 发给我",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply
        .messages
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
    assert!(
        reply.text.contains("未找到")
            || reply.text.contains("没有找到")
            || reply.text.contains("not found")
    );
    assert!(reply.text.contains("definitely_missing_named_file.txt"));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_returns_file_token_from_path_batch_after_read_rejections() {
    let state = test_state();
    let task = claimed_task("task-file-delivery-after-read-rejections");
    let tmp = TempDirGuard::new("file_delivery_path_batch_after_reject");
    let file = tmp.path().join("release_checklist.md");
    std::fs::write(&file, "release checklist").expect("write temp file");

    let mut route = scalar_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.wants_file_delivery = false;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file.display().to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let contract_error = "__RC_SKILL_ERROR__:{\"error_kind\":\"contract_action_rejected\",\"error_text\":\"action `system_basic.read_range` is rejected by contract `generic_delivery` (rejected_not_allowed)\",\"extra\":{\"action\":\"system_basic.read_range\",\"contract_match\":\"generic_delivery\",\"decision\":\"rejected_not_allowed\"},\"skill\":\"system_basic\"}";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(contract_error.to_string()),
        started_at: 1,
        finished_at: 2,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(contract_error.to_string()),
        started_at: 3,
        finished_at: 4,
    });
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        &serde_json::json!({
            "action": "path_batch_facts",
            "count": 1,
            "facts": [{
                "exists": true,
                "fact": {
                    "kind": "file",
                    "path": file.display().to_string(),
                    "resolved_path": file.display().to_string(),
                    "size_bytes": 17
                },
                "path": file.display().to_string()
            }],
            "include_missing": true
        })
        .to_string(),
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return file token");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, format!("FILE:{}", file.display()));
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_returns_not_found_for_run_cmd_not_found_delivery() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery-run-cmd");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("NOT_FOUND\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 /tmp/definitely-missing.txt 发给我",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    let summary = reply
        .messages
        .iter()
        .find(|message| crate::finalize::is_execution_summary_message(message))
        .expect("missing-file reply should include execution process");
    assert!(summary.contains("file not found"));
    assert!(
        reply.text.contains("未找到")
            || reply.text.contains("没有找到")
            || reply.text.contains("not found")
    );
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_returns_not_found_for_missing_path_facts_delivery() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery-path-facts");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_user_visible_respond = Some("FILE:/tmp/definitely-missing.txt".to_string());
    loop_state.delivery_messages = vec!["FILE:/tmp/definitely-missing.txt".to_string()];

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 /tmp/definitely-missing.txt 发给我",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply
        .messages
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
    assert!(
        reply.text.contains("未找到")
            || reply.text.contains("没有找到")
            || reply.text.contains("not found")
    );
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_keeps_missing_file_delivery_when_synthesis_is_non_token() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery-synthesis");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output =
        Some("文件 /tmp/definitely-missing.txt 不存在，无法发送。".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 /tmp/definitely-missing.txt 发给我，不要猜内容",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply.text.contains("/tmp/definitely-missing.txt"));
    assert!(
        reply.text.contains("未找到")
            || reply.text.contains("没有找到")
            || reply.text.contains("not found")
    );
    assert!(reply
        .messages
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}
