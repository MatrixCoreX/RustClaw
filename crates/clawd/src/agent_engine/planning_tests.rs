use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;

use super::{
    action_targets_config_edit, actions_use_ad_hoc_command_without_route_preferred_skill,
    active_task_append_current_locator_deterministic_plan_result,
    archive_list_auto_locator_deterministic_plan_result, archive_read_deterministic_plan_result,
    archive_unpack_deterministic_plan_result, broaden_default_read_range_for_structured_text,
    build_lightweight_skill_playbooks_text, build_lightweight_skill_quick_index_text,
    build_lightweight_tool_spec, can_fallback_to_initial_plan_after_repair_failure,
    classify_planning_prompt_class, compact_skill_playbook_from_prompt,
    content_excerpt_summary_auto_locator_deterministic_plan_result,
    contract_hint_preferred_action_deterministic_plan_result,
    directory_compare_locator_deterministic_plan_result,
    directory_entry_groups_auto_locator_deterministic_plan_result,
    directory_purpose_extension_inventory_deterministic_plan_result,
    directory_purpose_representative_reads_after_find_result,
    directory_tree_auto_locator_deterministic_plan_result, enforce_output_contract_tool_args,
    ensure_content_excerpt_summary_has_bounded_content, ensure_required_contract_block_present,
    existence_with_path_locator_deterministic_plan_result,
    explicit_command_deterministic_plan_result, file_facts_auto_locator_deterministic_plan_result,
    file_facts_auto_locator_observation_plan, file_paths_locator_deterministic_plan_result,
    fill_missing_read_range_path_from_route_locator,
    generic_directory_auto_locator_observation_plan,
    git_repository_state_deterministic_plan_result, has_pre_observation_structured_output_shape,
    inject_structural_extension_filter_for_directory_inventory,
    inject_synthesize_answer_for_bare_placeholder_respond,
    inline_json_transform_deterministic_plan_result, is_bare_last_output_placeholder,
    normalize_archive_basic_schema_aliases, normalize_fs_basic_schema_aliases,
    normalize_git_basic_schema_aliases, normalize_planned_actions,
    normalize_planned_actions_with_original, normalize_planned_actions_with_original_and_context,
    normalize_system_basic_schema_aliases, normalize_transform_schema_aliases,
    observation_only_plan_can_finalize_from_direct_output,
    package_manager_detect_deterministic_plan_result,
    package_manager_dry_run_deterministic_plan_result, plan_repair_reason,
    quantity_compare_pair_locator_deterministic_plan_result,
    registry_preferred_skill_names_for_route, repair_guard_config_default_path_for_invalid_locator,
    replace_file_delivery_respond_only_with_path_observation,
    replace_scalar_count_plan_with_count_inventory,
    replace_scalar_path_respond_only_with_auto_locator_observation,
    replace_workspace_synthesis_respond_only_plan,
    rewrite_archive_basic_short_archive_to_active_bound_target,
    rewrite_archive_pack_plan_to_archive_basic, rewrite_archive_unpack_run_cmd_to_archive_basic,
    rewrite_config_change_preview_to_config_edit_plan,
    rewrite_config_validation_read_plan_to_validate,
    rewrite_directory_entry_groups_tree_summary_to_list_dir,
    rewrite_docker_readonly_run_cmd_to_docker_basic, rewrite_extract_field_alias_args,
    rewrite_observed_terminal_synthesis_concrete_respond,
    rewrite_pre_observation_concrete_respond_to_placeholder,
    rewrite_process_ps_run_cmd_to_process_basic, rewrite_rustclaw_config_validation_to_guard,
    rewrite_service_status_plan_to_service_control,
    rewrite_sqlite_count_query_to_requested_schema_column,
    rewrite_sqlite_schema_version_plan_to_db_basic, rewrite_sqlite_table_listing_plan_to_db_basic,
    rewrite_sqlite_table_probe_to_requested_schema_value,
    rewrite_terminal_placeholder_respond_to_synthesize_answer,
    rewrite_terminal_synthesis_placeholder_respond,
    rewrite_unresolved_template_arg_multi_file_read_plan, round1_prompt_spec_for_class,
    scalar_content_auto_locator_deterministic_plan_result,
    scalar_content_auto_locator_observation_plan,
    scalar_path_auto_locator_deterministic_plan_result, scalar_path_auto_locator_observation_plan,
    scalar_path_directory_locator_search_deterministic_plan_result,
    service_status_deterministic_plan_result, should_force_actionable_plan_repair,
    strip_directory_read_range_after_inventory_dir, strip_file_lines_count_before_tail_read_range,
    strip_intermediate_synthesize_before_later_execution,
    strip_terminal_discussion_for_direct_skill_passthrough,
    strip_terminal_discussion_for_observed_finalize,
    strip_terminal_discussion_for_scalar_path_observation,
    strip_terminal_placeholder_respond_for_exact_listing_contract,
    strip_unresolved_template_reads_after_inventory_dir,
    structural_contract_deterministic_plan_overrides_literal_command_guard,
    structured_field_selectors, structured_keys_deterministic_plan_result, LoopState,
    PlanningPromptClass,
};
use crate::agent_engine::CLAWD_LITERAL_COMMAND_ARG;
use crate::{
    AgentAction, AgentRuntimeConfig, AppState, AskMode, ClaimedTask, IntentOutputContract,
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, PlanKind,
    ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind, SkillViewsSnapshot, ToolsPolicy,
    DEFAULT_AGENT_ID,
};
use serde_json::{json, Value};

fn planned_call<'a>(action: &'a AgentAction) -> Option<(&'a str, &'a Value)> {
    match action {
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        _ => None,
    }
}

fn planned_call_is(action: &AgentAction, name: &str, action_name: &str) -> bool {
    planned_call(action).is_some_and(|(tool, args)| {
        tool == name && args.get("action").and_then(Value::as_str) == Some(action_name)
    })
}

fn expect_planned_call<'a>(action: &'a AgentAction, name: &str, action_name: &str) -> &'a Value {
    let Some((tool, args)) = planned_call(action) else {
        panic!("expected {name}.{action_name} call, got {action:?}");
    };
    assert_eq!(tool, name);
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some(action_name)
    );
    args
}

#[test]
fn normalize_planned_actions_resolves_call_capability_before_policy_gate() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.list_entries".to_string(),
        args: json!({
            "path": ".",
            "names_only": true,
        }),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("path").and_then(Value::as_str), Some("."));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn normalize_planned_actions_resolves_action_ref_call_capability_before_policy_gate() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "fs_basic.read_text_range".to_string(),
        args: json!({
            "path": "scripts/nl_tests/fixtures/device_local/logs/app.log",
            "mode": "tail",
            "n": 20,
        }),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/logs/app.log")
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_i64), Some(20));
}

#[test]
fn normalize_planned_actions_keeps_unresolved_call_capability_for_verifier() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "unknown.example".to_string(),
        args: json!({}),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallCapability { capability, .. } if capability == "unknown.example"
    ));
}

#[test]
fn structured_text_read_range_without_bounds_reads_broader_context() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "prompts/schemas/direct_answer_gate.schema.json",
            "format": "text",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(500));
}

#[test]
fn structured_text_read_range_keeps_explicit_bounds() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "config.json",
            "start_line": 1,
            "end_line": 3,
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert!(args.get("mode").is_none());
    assert!(args.get("n").is_none());
}

#[test]
fn structured_text_full_mode_without_n_reads_broader_context() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "prompts/schemas/direct_answer_gate.schema.json",
            "mode": "full",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("full"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(500));
}

#[test]
fn structured_text_tail_mode_keeps_default_window() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "config.json",
            "mode": "tail",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert!(args.get("n").is_none());
}

#[test]
fn plain_text_read_range_keeps_default_bounds() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "README.md",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert!(args.get("mode").is_none());
    assert!(args.get("n").is_none());
}

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
            "clawd_planning_{prefix}_{}_{}",
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
                skills_list: Arc::new(HashSet::new()),
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

fn test_state_with_enabled_skills(skills: &[&str]) -> AppState {
    let state = test_state();
    let enabled: HashSet<String> = skills.iter().map(|skill| (*skill).to_string()).collect();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: None,
        skills_list: Arc::new(enabled),
    });
    state
}

fn action_capability_and_action<'a>(
    action: &'a AgentAction,
    capability: &str,
    action_name: &str,
) -> Option<&'a Value> {
    match action {
        AgentAction::CallSkill { skill, args } if skill == capability => Some(args),
        AgentAction::CallTool { tool, args } if tool == capability => Some(args),
        _ => None,
    }
    .filter(|args| args.get("action").and_then(Value::as_str) == Some(action_name))
}

fn test_state_with_registry() -> AppState {
    let state = test_state();
    let registry_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(HashSet::new()),
    });
    state
}

fn test_task() -> ClaimedTask {
    ClaimedTask {
        task_id: "test-task".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn base_route_result() -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    }
}

#[test]
fn sqlite_table_listing_route_rewrites_text_read_plan_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/tmp/app.sqlite",
                "command": "sqlite3 /tmp/app.sqlite \"SELECT name FROM sqlite_master WHERE type='table';\""
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_sqlite_table_listing_plan_to_db_basic(
        Some(&route),
        Some("/tmp/app.sqlite"),
        false,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
    assert!(matches!(rewritten[2], AgentAction::Respond { .. }));
}

#[test]
fn sqlite_binary_text_read_fallback_rewrites_to_db_basic_list_tables_without_semantic_kind() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/tmp/app.sqlite",
                "mode": "head",
                "n": 120
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten = rewrite_sqlite_table_listing_plan_to_db_basic(
        Some(&route),
        Some("/tmp/app.sqlite"),
        false,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(Value::as_str),
                Some("/tmp/app.sqlite")
            );
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn existence_path_summary_plan_inserts_bounded_content_observation() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["/tmp/rustclaw.service"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "check service file and summarize its purpose",
        Some("/tmp/rustclaw.service"),
        actions,
    );

    assert!(normalized.iter().any(|action| {
        action_capability_and_action(action, "fs_basic", "read_text_range").is_some_and(|args| {
            args.get("path").and_then(Value::as_str) == Some("/tmp/rustclaw.service")
        })
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn existence_path_summary_metadata_placeholder_does_not_force_file_read() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_hint = "data/rustclaw.db".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["/tmp/rustclaw.db"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "文件存在，大小为 {{size}} 字节。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "check whether the file exists and report its size",
        Some("/tmp/rustclaw.db"),
        actions,
    );

    assert!(!normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        )
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string()]
                    || evidence_refs == &vec!["last_output".to_string()]
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn existence_with_path_metadata_batch_answer_does_not_force_content_repair() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "README.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["/home/guagua/rustclaw/README.md"],
                "include_missing": true,
                "fields": ["exists", "size"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn filename_path_metadata_answer_does_not_force_content_repair_for_generic_contract() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["/home/guagua/rustclaw/rustclaw.service"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn sqlite_table_names_route_rewrites_system_basic_action_alias_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableNamesOnly;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "sqlite_table_names",
            "path": "/tmp/app.sqlite"
        }),
    }];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_table_listing_route_rewrites_text_field_extract_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/app.sqlite",
            "field_path": "sqlite_master.name"
        }),
    }];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_database_kind_judgment_rewrites_run_cmd_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteDatabaseKindJudgment;
    route.output_contract.locator_hint = "/tmp/db-basic-contract.sqlite".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "run_cmd",
                "command": "sqlite3 /tmp/db-basic-contract.sqlite \".tables\""
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/db-basic-contract.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn sqlite_table_listing_preserves_explicit_literal_run_cmd() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "sqlite3 /tmp/app.sqlite '.tables'"}),
    }];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("sqlite3 /tmp/app.sqlite '.tables'")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn sqlite_schema_version_extract_field_rewrites_to_db_basic_pragma() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/app.sqlite",
            "field_path": "schema_version"
        }),
    }];

    let rewritten =
        rewrite_sqlite_schema_version_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("schema_version")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_schema_version_extract_fields_rewrites_from_action_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_fields",
            "path": "/tmp/app.db",
            "field_paths": ["schema_version"]
        }),
    }];

    let rewritten = rewrite_sqlite_schema_version_plan_to_db_basic(None, None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("schema_version")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.db")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_schema_version_route_rewrites_binary_text_read_to_db_basic_pragma() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteSchemaVersion;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/tmp/app.sqlite",
                "mode": "head",
                "n": 100
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten =
        rewrite_sqlite_schema_version_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("schema_version")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn sqlite_count_query_rewrites_to_requested_schema_column_when_count_conflicts_with_column_intent()
{
    let tmp = TempDirGuard::new("sqlite_count_column_rewrite");
    let db_path = tmp.path.join("orders.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "CREATE TABLE orders (id INTEGER, user_id INTEGER, amount REAL, status TEXT)",
        [],
    )
    .expect("create table");
    let mut route = base_route_result();
    route.resolved_intent =
        "Read the amount of orders with status='pending' from the SQLite database".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "sqlite_query",
            "db_path": db_path,
            "sql": "SELECT COUNT(*) FROM orders WHERE status='pending';"
        }),
    }];

    let rewritten = rewrite_sqlite_count_query_to_requested_schema_column(
        Some(&route),
        "Read the pending order amount.",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "sqlite_query");
    assert_eq!(
        args.get("sql").and_then(Value::as_str),
        Some(r#"SELECT "amount" FROM "orders" WHERE status='pending'"#)
    );
}

#[test]
fn sqlite_count_query_does_not_rewrite_scalar_count_contract() {
    let tmp = TempDirGuard::new("sqlite_count_contract_preserve");
    let db_path = tmp.path.join("users.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute("CREATE TABLE users (id INTEGER, name TEXT)", [])
        .expect("create table");
    let mut route = base_route_result();
    route.resolved_intent = "Count rows in the users table".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "sqlite_query",
            "db_path": db_path,
            "sql": "SELECT COUNT(*) FROM users;"
        }),
    }];

    let rewritten = rewrite_sqlite_count_query_to_requested_schema_column(
        Some(&route),
        "How many users are stored?",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "sqlite_query");
    assert_eq!(
        args.get("sql").and_then(Value::as_str),
        Some("SELECT COUNT(*) FROM users;")
    );
}

#[test]
fn sqlite_table_probe_rewrites_to_requested_schema_value_query() {
    let tmp = TempDirGuard::new("sqlite_table_probe_value_rewrite");
    let db_path = tmp.path.join("orders.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "CREATE TABLE orders (id INTEGER, user_id INTEGER, amount REAL, status TEXT)",
        [],
    )
    .expect("create table");
    conn.execute(
        "INSERT INTO orders (id, user_id, amount, status) VALUES (1, 1, 7.5, 'pending')",
        [],
    )
    .expect("insert pending order");
    let mut route = base_route_result();
    route.resolved_intent =
        "Read the amount from the order table where status is pending".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "list_tables",
            "db_path": db_path,
        }),
    }];

    let rewritten = rewrite_sqlite_table_probe_to_requested_schema_value(
        Some(&route),
        "Read the pending order amount.",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "sqlite_query");
    assert_eq!(
        args.get("sql").and_then(Value::as_str),
        Some(r#"SELECT "amount" FROM "orders" WHERE "status" = 'pending'"#)
    );
}

#[test]
fn sqlite_table_probe_keeps_table_listing_contract() {
    let tmp = TempDirGuard::new("sqlite_table_probe_listing_preserve");
    let db_path = tmp.path.join("orders.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute("CREATE TABLE orders (id INTEGER, status TEXT)", [])
        .expect("create table");
    let mut route = base_route_result();
    route.resolved_intent = "List the tables in the database".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableNamesOnly;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "list_tables",
            "db_path": db_path,
        }),
    }];

    let rewritten = rewrite_sqlite_table_probe_to_requested_schema_value(
        Some(&route),
        "List the tables in the database.",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "list_tables");
    assert!(args.get("sql").is_none());
}

#[test]
fn file_delivery_respond_only_gets_path_observation_before_file_token() {
    let tmp = TempDirGuard::new("file_delivery_observation");
    let file_path = tmp.path.join("service_notes.md");
    fs::write(&file_path, "notes\n").expect("write file");
    let state = test_state();
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file_path.display().to_string();
    let token = format!("FILE:{}", file_path.display());
    let actions = vec![AgentAction::Respond { content: token }];

    let rewritten = replace_file_delivery_respond_only_with_path_observation(
        &state,
        Some(&route),
        &LoopState::default(),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
        }
        other => panic!("expected path observation, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::Respond { .. }));
}

#[test]
fn generated_file_write_delivery_appends_file_token() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("generated_file_delivery_append_token");
    state.skill_rt.workspace_root = tmp.path.clone();
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "write_text",
            "path": "tmp/对抗测试_笔记.txt",
            "content": "adversarial v1"
        }),
    }];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "create and deliver a file",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(planned_call_is(&rewritten[0], "fs_basic", "write_text"));
    let expected = format!("FILE:{}", tmp.path.join("tmp/对抗测试_笔记.txt").display());
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &expected
    ));
}

#[test]
fn generated_file_write_delivery_replaces_non_file_terminal_respond() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("generated_file_delivery_replace_respond");
    state.skill_rt.workspace_root = tmp.path.clone();
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": "tmp/note.txt",
                "content": "ok"
            }),
        },
        AgentAction::Respond {
            content: "created".to_string(),
        },
    ];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "create and deliver a file",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    let expected = format!("FILE:{}", tmp.path.join("tmp/note.txt").display());
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &expected
    ));
}

#[test]
fn file_delivery_terminal_token_is_not_rewritten_to_content_synthesis() {
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/LICENSE.zh-CN.md".to_string();
    route.output_contract.requires_content_evidence = true;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path":"/tmp/LICENSE.zh-CN.md"}),
        },
        AgentAction::Respond {
            content: "FILE:/tmp/LICENSE.zh-CN.md".to_string(),
        },
    ];

    let rewritten = rewrite_pre_observation_concrete_respond_to_placeholder(
        Some(&route),
        &LoopState::default(),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(rewritten[0], AgentAction::CallSkill { .. }));
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == "FILE:/tmp/LICENSE.zh-CN.md"
    ));
}

#[test]
fn file_token_respond_survives_even_when_delivery_contract_is_missing() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/LICENSE.zh-CN.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path":"/tmp/LICENSE.zh-CN.md"}),
        },
        AgentAction::Respond {
            content: "FILE:/tmp/LICENSE.zh-CN.md".to_string(),
        },
    ];

    let rewritten = rewrite_pre_observation_concrete_respond_to_placeholder(
        Some(&route),
        &LoopState::default(),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == "FILE:/tmp/LICENSE.zh-CN.md"
    ));
}

#[test]
fn archive_unpack_route_rewrites_run_cmd_unzip_to_archive_basic() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/bundle.zip | /tmp/out".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "unzip \"/tmp/bundle.zip\" -d \"/tmp/out\""
        }),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("unpack")
            );
            assert_eq!(
                args.get("archive").and_then(|value| value.as_str()),
                Some("/tmp/bundle.zip")
            );
            assert_eq!(
                args.get("dest").and_then(|value| value.as_str()),
                Some("/tmp/out")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_unpack_route_rewrites_archive_read_plan_to_unpack() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/bundle.zip | /tmp/out".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "read",
            "archive": "/tmp/bundle.zip",
            "member": "/tmp/out",
        }),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("unpack")
            );
            assert_eq!(
                args.get("archive").and_then(|value| value.as_str()),
                Some("/tmp/bundle.zip")
            );
            assert_eq!(
                args.get("dest").and_then(|value| value.as_str()),
                Some("/tmp/out")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn content_excerpt_archive_member_read_is_not_rewritten_to_unpack() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "read",
            "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
            "member": "notes.txt",
        }),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), false, actions);

    let args = expect_planned_call(&rewritten[0], "archive_basic", "read");
    assert_eq!(
        args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_unpack_contract_plans_direct_unpack_without_model_plan() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_unpack_deterministic_plan_result(
        "unpack archive",
        &state,
        Some(&route),
        &loop_state,
    )
    .expect("archive unpack deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "unpack");
    assert_eq!(
        args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
    assert_eq!(
        args.get("dest").and_then(Value::as_str),
        Some("tmp/contract_matrix_unpacked")
    );
}

#[test]
fn archive_unpack_preserves_explicit_literal_run_cmd() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/input.zip | /tmp/out".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "unzip /tmp/input.zip -d /tmp/out"}),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("unzip /tmp/input.zip -d /tmp/out")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn archive_pack_route_rewrites_probe_only_plan_to_archive_basic() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": [
                    "/home/guagua/rustclaw/scripts/skill_calls",
                    "/home/guagua/rustclaw/tmp/nl_archive_case_en.zip"
                ]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "Unable to create the zip archive.".to_string(),
        },
    ];

    let rewritten = rewrite_archive_pack_plan_to_archive_basic(Some(&route), false, actions);

    assert_eq!(rewritten.len(), 2);
    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("pack")
            );
            assert_eq!(
                args.get("source").and_then(|value| value.as_str()),
                Some("scripts/skill_calls")
            );
            assert_eq!(
                args.get("archive").and_then(|value| value.as_str()),
                Some("tmp/nl_archive_case_en.zip")
            );
            assert_eq!(
                args.get("format").and_then(|value| value.as_str()),
                Some("zip")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn archive_pack_route_rewrites_archive_list_plan_to_pack() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: json!({
                "action": "list",
                "archive": "tmp/nl_archive_case_en.zip",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_archive_pack_plan_to_archive_basic(Some(&route), false, actions);

    assert_eq!(rewritten.len(), 2);
    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("pack"));
            assert_eq!(
                args.get("source").and_then(Value::as_str),
                Some("scripts/skill_calls")
            );
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("tmp/nl_archive_case_en.zip")
            );
        }
        other => panic!("expected archive_basic pack action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn archive_pack_preserves_explicit_literal_run_cmd() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/source | /tmp/source.tgz".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "tar -czf /tmp/source.tgz /tmp/source"}),
    }];

    let rewritten = rewrite_archive_pack_plan_to_archive_basic(Some(&route), true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("tar -czf /tmp/source.tgz /tmp/source")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn archive_basic_pack_alias_args_normalize_to_contract() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls -> tmp/nl_archive_case_en.zip".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "pack",
            "source_path": "/home/guagua/rustclaw/scripts/skill_calls",
            "archive_path": "/home/guagua/rustclaw/tmp/nl_archive_case_en.zip",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(Some(&route), actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("source").and_then(Value::as_str),
                Some("/home/guagua/rustclaw/scripts/skill_calls")
            );
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("/home/guagua/rustclaw/tmp/nl_archive_case_en.zip")
            );
            assert_eq!(args.get("format").and_then(Value::as_str), Some("zip"));
            assert!(args.get("source_path").is_none());
            assert!(args.get("archive_path").is_none());
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn config_change_preview_read_plan_rewrites_to_config_edit_plan() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "configs/config.toml",
                "field_path": "skills.skill_switches",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten = rewrite_config_change_preview_to_config_edit_plan(
            Some(&route),
            "只生成变更计划，不要实际修改：把 configs/config.toml 里的 skills.skill_switches.affected100_probe 设置为 true，并说明会改哪里",
            Some("configs/config.toml"),
            actions,
        );

    assert_eq!(rewritten.len(), 1);
    let args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.affected100_probe")
    );
    assert_eq!(args.get("value").and_then(Value::as_bool), Some(true));
}

#[test]
fn unrequested_config_edit_is_stripped_from_text_rewrite_followup() {
    let state = test_state_with_enabled_skills(&["config_edit", "synthesize_answer"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent = "rewrite_active_text_style_only".to_string();
    route.route_reason = "style_transform_without_config_anchor".to_string();
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_edit".to_string(),
            args: json!({
                "action": "plan_config_change",
                "path": "configs/config.toml",
                "field_path": "build-all.sh",
                "value": 1
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "rewritten_text_body".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "active_task_id=summary\nprevious_output=configs/config.toml build-all.sh 1\ncurrent_instruction=rewrite_style_only",
            Some("rewrite_style_only"),
            None,
            actions,
        );

    assert!(
        !normalized.iter().any(action_targets_config_edit),
        "normalized actions: {normalized:?}"
    );
    assert!(matches!(
        normalized.as_slice(),
        [AgentAction::Respond { content }] if content == "rewritten_text_body"
    ));
}

#[test]
fn requested_config_edit_plan_is_preserved_by_structural_anchors() {
    let state = test_state_with_enabled_skills(&["config_edit"]);
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "plan_config_change",
            "path": "configs/config.toml",
            "field_path": "server.port",
            "value": 8787
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "configs/config.toml server.port 8787",
        Some("configs/config.toml server.port 8787"),
        None,
        actions,
    );

    assert!(
        normalized.iter().any(action_targets_config_edit),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn rustclaw_config_problem_validation_rewrites_to_guard_config() {
    let mut route = base_route_result();
    route.resolved_intent =
        "Validate the selected RustClaw config with semantic guard profile.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": "configs/config.toml",
            "format": "toml",
            "validation_profile": "rustclaw_semantic_guard",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    let args = expect_planned_call(&rewritten[0], "config_edit", "guard_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
}

#[test]
fn rustclaw_config_syntax_only_validation_keeps_validate_action() {
    let mut route = base_route_result();
    route.resolved_intent = "Validate TOML syntax only.".to_string();
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": "configs/config.toml",
            "format": "toml",
            "validation_profile": "syntax_only",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    expect_planned_call(&rewritten[0], "config_basic", "validate");
}

#[test]
fn plain_main_config_validation_rewrites_to_guard_when_not_syntax_contract() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "validate",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "check main config for obvious configuration issues",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_edit", "guard_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
}

#[test]
fn guard_config_with_invalid_product_locator_uses_main_config_default() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/rustclaw".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "config_basic".to_string(),
        args: json!({
            "action": "guard_rustclaw_config",
            "path": "/home/guagua/rustclaw/rustclaw",
        }),
    }];

    let normalized = repair_guard_config_default_path_for_invalid_locator(
        Some(&route),
        Some("/home/guagua/rustclaw/rustclaw"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
}

#[test]
fn config_validation_contract_rewrites_broad_read_to_validate() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "configs/config.toml",
                "mode": "head",
                "n": 500,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_config_validation_read_plan_to_validate(Some(&route), None, actions);

    let args = expect_planned_call(&rewritten[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn config_validation_contract_normalizes_tool_read_to_validate() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.resolved_intent =
        "Validate TOML syntax of configs/config.toml and answer pass or fail".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "mode": "head",
                "n": 120,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Validate only the TOML syntax of configs/config.toml and answer pass or fail.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_validation_contract_normalizes_legacy_system_validate_structured_to_validate() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml".to_string();
    route.resolved_intent =
        "Validate app_config.toml and briefly say whether it is readable".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "validate_structured",
                "path": "scripts/nl_tests/fixtures/device_local/configs/app_config.toml",
                "format": "toml",
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "验证 scripts/nl_tests/fixtures/device_local/configs/app_config.toml 是否是可读配置，并简短说明结果。",
            None,
            actions,
        );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/configs/app_config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized.iter().all(|action| !planned_call_is(
            action,
            "system_basic",
            "validate_structured"
        )),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_validation_contract_normalizes_config_field_read_to_validate() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.resolved_intent =
        "Validate TOML syntax of configs/config.toml and answer pass or fail".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "field_path": "memory",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Validate only the TOML syntax of configs/config.toml and answer pass or fail.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "read_field")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn unrequested_path_like_config_field_read_rewrites_to_validate() {
    let root = TempDirGuard::new("unrequested_path_like_config_field");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("app_config.toml");
    fs::write(&config_path, "[app]\nname = \"demo\"\n").expect("write config");
    let config_text = config_path.display().to_string();

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_text.clone();
    route.resolved_intent = format!("Validate TOML syntax of {config_text}.");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": config_text,
            "field_path": "no_such_note.md",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Validate only whether configs/app_config.toml can be parsed as TOML.",
        Some(config_path.display().to_string().as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "read_field")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn explicit_path_like_config_field_read_is_preserved_when_user_mentions_field() {
    let root = TempDirGuard::new("explicit_path_like_config_field");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("app_config.toml");
    fs::write(&config_path, "[app]\nname = \"demo\"\n").expect("write config");
    let config_text = config_path.display().to_string();

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_text.clone();
    route.resolved_intent = format!("Read field no_such_note.md from {config_text}.");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": config_text,
            "field_path": "no_such_note.md",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read field no_such_note.md from configs/app_config.toml.",
        Some(config_path.display().to_string().as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("no_such_note.md")
    );
}

#[test]
fn rustclaw_config_section_header_field_reads_rewrite_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "list_keys",
                "path": "/home/guagua/rustclaw/configs/config.toml",
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_fields",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "field_paths": ["[server]", "[security]", "[auth]"],
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_2".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "Inspect RustClaw configuration file configs/config.toml for security or risk-related settings and present only the important findings.",
            Some("/home/guagua/rustclaw/configs/config.toml"),
            actions,
        );

    let args = expect_planned_call(&normalized[0], "config_edit", "guard_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "read_fields")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_risk_assessment_rewrites_key_listing_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": "/home/guagua/rustclaw/configs/config.toml",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Structured RustClaw config risk assessment.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_edit", "guard_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "list_keys")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_risk_assessment_rewrites_file_head_read_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Structured RustClaw config risk assessment.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_edit", "guard_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn rustclaw_main_config_content_excerpt_broad_read_rewrites_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Summarize the main config after observing current-task evidence.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_edit", "guard_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
}

#[test]
fn rustclaw_main_config_content_excerpt_tail_read_stays_bounded_read() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "mode": "tail",
            "n": 5,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Summarize the bounded tail excerpt.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(5));
}

#[test]
fn config_risk_assessment_rewrites_registry_head_read_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/configs/skills_registry.toml",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Structured RustClaw registry risk assessment.",
        Some("/home/guagua/rustclaw/configs/skills_registry.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_edit", "guard_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/skills_registry.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn scalar_structured_field_contract_rewrites_broad_read_to_read_field() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "crates/clawd/Cargo.toml".to_string();
    route.resolved_intent =
        "Read package.version from crates/clawd/Cargo.toml and output only the value.".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "crates/clawd/Cargo.toml",
                "mode": "head",
                "n": 500,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read package.version from crates/clawd/Cargo.toml and output only the value.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("crates/clawd/Cargo.toml")
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("package.version")
    );
}

#[test]
fn scalar_structured_field_contract_infers_single_field_from_structural_candidate() {
    let root = TempDirGuard::new("structured_scalar_field_candidate_plan");
    let root_package = root.path.join("package.json");
    fs::write(&root_package, r#"{"dependencies":{"vite":"latest"}}"#).expect("write root");
    let fixture_dir = root.path.join("fixtures");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let fixture_package = fixture_dir.join("package.json");
    fs::write(
        &fixture_package,
        r#"{"name":"rustclaw-nl-fixture","dependencies":{}}"#,
    )
    .expect("write fixture");
    let root_package_path = root_package.display().to_string();
    let fixture_package_path = fixture_package.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.route_reason =
        "llm_semantic_contract_repair:single_path_field_extraction_semantic_kind_none_is_valid"
            .to_string();
    route.resolved_intent =
        "Extract and output only the value of the name field from package.json".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": root_package_path,
                "mode": "head",
                "n": 500,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "package.json",
        Some(&root_package.display().to_string()),
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(fixture_package_path.as_str())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn scalar_structured_field_contract_rewrites_key_listing_to_read_field() {
    let root = TempDirGuard::new("structured_scalar_field_list_keys_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config = config_dir.join("app_config.toml");
    fs::write(&config, "[app]\nport = 8787\n").expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.resolved_intent = format!("Read app.port from {config_path} and output only the value.");
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "list_keys",
                "path": config_path,
                "max_keys": 1000,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read app.port from configs/app_config.toml and output only the value.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("app.port")
    );
}

#[test]
fn scalar_structured_keys_repair_marker_rewrites_key_listing_to_read_field() {
    let root = TempDirGuard::new("structured_keys_scalar_marker_plan");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"fixture","dependencies":{}}"#).expect("write package");
    let package_path = package.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.route_reason =
        "llm_semantic_contract_repair:structured_keys_scalar_response_requires_field_value"
            .to_string();
    route.resolved_intent = "Extract name field value from package.json".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": package_path,
            "max_keys": 1000,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "去 package.json 里把 name 的值回给我",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn structured_multi_field_contract_rewrites_broad_read_to_read_fields() {
    let root = TempDirGuard::new("structured_multi_field_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config = config_dir.join("app_config.toml");
    fs::write(
        &config,
        r#"[app]
name = "RustClaw NL Fixture"

[paths]
docs_dir = "docs"
logs_dir = "logs"
db_path = "data/test_contract.sqlite"
"#,
    )
    .expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.resolved_intent =
        "Return paths.logs_dir and paths.db_path from app_config.toml.".to_string();
    assert_eq!(
            structured_field_selectors(
                &route,
                "scripts/nl_tests/fixtures/device_local/configs/app_config.toml 의 paths.logs_dir 와 paths.db_path 값만 알려줘.",
                None,
                Some(&config_path),
            ),
            vec!["paths.logs_dir".to_string(), "paths.db_path".to_string()]
        );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": config_path,
                "mode": "head",
                "n": 120,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "scripts/nl_tests/fixtures/device_local/configs/app_config.toml 의 paths.logs_dir 와 paths.db_path 값만 알려줘.",
            None,
            actions,
        );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_fields");
    let field_paths = args
        .get("field_paths")
        .and_then(Value::as_array)
        .expect("field_paths");
    assert_eq!(
        field_paths,
        &vec![json!("paths.logs_dir"), json!("paths.db_path")]
    );
}

#[test]
fn structured_multi_field_rewrite_ignores_background_filename_tokens() {
    let root = TempDirGuard::new("structured_multi_field_background_paths");
    let schema_dir = root.path.join("prompts/schemas");
    fs::create_dir_all(&schema_dir).expect("create schema dir");
    let schema = schema_dir.join("intent_normalizer.schema.json");
    fs::write(
        &schema,
        r#"{"type":"object","properties":{"kind":{"type":"string"}}}"#,
    )
    .expect("write schema");
    let schema_path = schema.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = schema_path.clone();
    route.resolved_intent =
        "List schema files, find the largest, and summarize its purpose.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": schema_path,
            "mode": "head",
            "n": 50,
        }),
    }];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "列出 prompts/schemas 下的 json 文件，找最大的并总结它描述什么对象。",
            Some(
                "STABLE_FACTS: 甲文件指向 docs/release_checklist.md，另一个文件是 docs/service_notes.md",
            ),
            actions,
        );

    assert!(
        normalized
            .iter()
            .any(|action| planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
    assert!(
        normalized.iter().all(
            |action| !planned_call_is(action, "config_basic", "read_fields")
                && !planned_call_is(action, "config_basic", "validate")
        ),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn structured_multi_field_contract_rewrites_key_listing_to_read_fields() {
    let root = TempDirGuard::new("structured_multi_field_list_keys_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config = config_dir.join("app_config.toml");
    fs::write(
        &config,
        r#"[paths]
logs_dir = "logs"
db_path = "data/test_contract.sqlite"
"#,
    )
    .expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.resolved_intent = format!("Return paths.logs_dir and paths.db_path from {config_path}.");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": config_path,
            "max_keys": 1000,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Return paths.logs_dir and paths.db_path from configs/app_config.toml.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_fields");
    let field_paths = args
        .get("field_paths")
        .and_then(Value::as_array)
        .expect("field_paths");
    assert_eq!(
        field_paths,
        &vec![json!("paths.logs_dir"), json!("paths.db_path")]
    );
}

#[test]
fn structured_identity_scalar_contract_rewrites_broad_read_to_read_field() {
    let root = TempDirGuard::new("structured_identity_field_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "fs_basic"
group = "filesystem"
planner_kind = "tool"

[[skills]]
name = "archive_basic"
group = "archive"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.resolved_intent =
        "Read skills_registry.toml and return the group value for archive_basic.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": registry_path,
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "configs/skills_registry.toml で archive_basic の group だけ答えて。",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("archive_basic.group")
    );
}

#[test]
fn structured_identity_presence_contract_rewrites_stat_to_read_field() {
    let root = TempDirGuard::new("structured_identity_presence_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "fs_basic"
group = "filesystem"
planner_kind = "tool"

[[skills]]
name = "archive_basic"
group = "archive"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.route_reason = "structured_identifier_presence_requires_content_evidence".to_string();
    route.resolved_intent =
        "Read skills_registry.toml and answer whether fs_basic is registered.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "paths": [registry_path],
            "include_missing": true,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read skills_registry.toml and answer whether fs_basic is registered.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("fs_basic.name")
    );
}

#[test]
fn structured_identity_presence_contract_rewrites_validate_to_read_field() {
    let root = TempDirGuard::new("structured_identity_presence_validate_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "fs_basic"
group = "filesystem"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.route_reason = "structured_identifier_presence_requires_content_evidence".to_string();
    route.resolved_intent =
        "Read skills_registry.toml and answer whether fs_basic is registered.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": registry_path,
            "format": "toml",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read skills_registry.toml and answer whether fs_basic is registered.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("fs_basic.name")
    );
}

#[test]
fn rustclaw_config_validation_without_profile_keeps_validate_action() {
    let mut route = base_route_result();
    route.resolved_intent =
        "Legacy risk/problem wording in route text must not trigger runtime rewrites.".to_string();
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    expect_planned_call(&rewritten[0], "config_basic", "validate");
}

#[test]
fn rustclaw_config_guard_profile_without_locator_keeps_validate_action() {
    let mut route = base_route_result();
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "format": "toml",
            "validation_profile": "rustclaw_semantic_guard",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    expect_planned_call(&rewritten[0], "config_basic", "validate");
}

#[test]
fn archive_basic_pack_output_alias_normalizes_to_archive() {
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "pack",
            "source": "scripts/skill_calls",
            "output": "tmp/nl_archive_case.zip",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(None, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("tmp/nl_archive_case.zip")
            );
            assert_eq!(args.get("format").and_then(Value::as_str), Some("zip"));
            assert!(args.get("output").is_none());
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_list_path_alias_normalizes_to_archive_contract() {
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "list",
            "path": "/tmp/rustclaw_archive_nl_case/sample.tgz",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(None, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("/tmp/rustclaw_archive_nl_case/sample.tgz")
            );
            assert!(args.get("path").is_none());
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_read_action_preserves_member_contract() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "read",
            "path": archive,
            "entry": "notes.txt",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(None, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("read"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
            assert_eq!(
                args.get("member").and_then(Value::as_str),
                Some("notes.txt")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_short_list_archive_uses_active_bound_target() {
    let bound_target = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "list",
            "archive": "test_bundle.zip",
        }),
    }];
    let plan_context = format!(
            "### ACTIVE_EXECUTION_ANCHOR\nfollowup_op_kind: Read\nfollowup_bound_target: {bound_target}\nobserved_bound_target: {bound_target}"
        );

    let rewritten =
        rewrite_archive_basic_short_archive_to_active_bound_target(Some(&plan_context), actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some(bound_target)
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn package_manager_detect_uses_deterministic_skill_plan() {
    let state = test_state_with_enabled_skills(&["package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::PackageManagerDetection;
    let loop_state = LoopState::new(1);

    let plan = package_manager_detect_deterministic_plan_result(
        &state,
        "detect package manager",
        Some(&route),
        &loop_state,
        Some("/workspace/UI"),
    )
    .expect("package manager detection should use deterministic plan");

    assert_eq!(plan.steps.len(), 3);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "package_manager", "detect");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("detect"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/workspace/UI")
    );
}

#[test]
fn contract_hint_preferred_run_cmd_uses_machine_hint_not_request_words() {
    let state = test_state_with_enabled_skills(&["run_cmd", "package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::PackageManagerDetection;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let request =
            "arbitrary multilingual surface\n[CONTRACT_TEST_HINT]\npreferred_action_ref=run_cmd\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "detect package manager",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("machine hint should select run_cmd");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert!(plan.steps[0]
        .args
        .get("command")
        .and_then(Value::as_str)
        .is_some());
}

#[test]
fn contract_hint_preferred_run_cmd_sqlite_uses_structured_locator() {
    let state = test_state_with_enabled_skills(&["run_cmd", "db_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteDatabaseKindJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=run_cmd\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "inspect sqlite database kind",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("machine hint should select sqlite run_cmd probe");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    let command = plan.steps[0]
        .args
        .get("command")
        .and_then(Value::as_str)
        .expect("command");
    assert!(command.contains("sqlite3"));
    assert!(command.contains("test_contract.sqlite"));
    assert!(command.contains(".tables"));
}

#[test]
fn contract_hint_preferred_db_basic_does_not_claim_structured_keys_config_file() {
    let root = TempDirGuard::new("contract_hint_structured_keys_db_basic");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let state = test_state_with_enabled_skills(&["config_basic", "db_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=db_basic\n[/CONTRACT_TEST_HINT]";

    assert!(contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list structured keys",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&config_path),
    )
    .is_none());

    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list structured keys",
        "list structured keys",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys should fall back to config_basic list_keys");
    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

#[test]
fn contract_hint_workspace_summary_list_dir_prefers_text_excerpt_evidence() {
    let root = TempDirGuard::new("contract_hint_workspace_summary_list_dir");
    fs::write(
        root.path.join("README.md"),
        "# Fixture\n\nThis directory contains local test fixtures.",
    )
    .expect("write README");
    let root_path = root.path.display().to_string();
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    let request =
        "[CONTRACT_TEST_HINT]\npreferred_action_ref=fs_basic.list_dir\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize workspace",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&root_path),
    )
    .expect("workspace summary should use readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root.path.join("README.md").display().to_string().as_str())
    );
}

#[test]
fn contract_hint_workspace_summary_git_basic_prefers_text_excerpt_evidence() {
    let root = TempDirGuard::new("contract_hint_workspace_summary_git_basic");
    fs::write(
        root.path.join("README.md"),
        "# Fixture\n\nThis directory contains local test fixtures.",
    )
    .expect("write README");
    let root_path = root.path.display().to_string();
    let state = test_state_with_enabled_skills(&["fs_basic", "git_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=git_basic\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize workspace",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&root_path),
    )
    .expect("workspace summary should use readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root.path.join("README.md").display().to_string().as_str())
    );
}

#[test]
fn contract_hint_generic_path_content_stat_paths_prefers_text_excerpt_evidence() {
    let root = TempDirGuard::new("contract_hint_generic_path_content_stat_paths");
    let doc_path = root.path.join("release_checklist.md");
    fs::write(
        &doc_path,
        "# Release Checklist\n\n- Verify config loading\n- Check recent logs\n",
    )
    .expect("write doc");
    let doc_path = doc_path.display().to_string();
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = doc_path.clone();
    let request =
        "[CONTRACT_TEST_HINT]\npreferred_action_ref=fs_basic.stat_paths\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize file",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&doc_path),
    )
    .expect("generic file summary should use readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(doc_path.as_str())
    );
}

#[test]
fn contract_hint_preferred_fs_stat_paths_uses_locator_contract() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/package.json".to_string();
    let request =
        "[CONTRACT_TEST_HINT]\npreferred_action_ref=fs_basic.stat_paths\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "return path",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("machine hint should select fs_basic.stat_paths");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("stat_paths")
    );
}

#[test]
fn contract_hint_scalar_equality_without_locator_falls_back_to_git_branch() {
    let state = test_state_with_enabled_skills(&["fs_basic", "git_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=recent_scalar_equality_check\ncandidate_wrong_action_ref=db_basic\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check scalar equality",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("matrix fallback should select git_basic when no path locator exists");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "git_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("current_branch")
    );
}

#[test]
fn contract_hint_matrix_preferred_workspace_summary_reads_text_evidence() {
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    let root = TempDirGuard::new("contract_hint_workspace_summary");
    let fixture_dir = root.path.join("fixture_project");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    fs::write(
        fixture_dir.join("README.md"),
        "# Fixture Project\n\nA small local project used by contract tests.\n",
    )
    .expect("write readme");
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "fixture_project".to_string();
    let request =
        "[CONTRACT_TEST_HINT]\nsemantic_kind=workspace_project_summary\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize project",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("matrix preferred action should select readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("read_text_range")
    );
    assert!(plan.steps[0]
        .args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("fixture_project/README.md")));
}

#[test]
fn contract_hint_matrix_preferred_docker_logs_reads_container_candidates_first() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DockerLogs;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=docker_logs\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "inspect docker logs",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("docker logs contract should first gather candidate containers");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "docker_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("ps")
    );
}

#[test]
fn contract_hint_matrix_existence_summary_reads_stat_and_content_from_route_context() {
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    let root = TempDirGuard::new("contract_hint_existence_summary");
    let fixture = root.path.join("package.json");
    fs::write(
        &fixture,
        r#"{"name":"rustclaw-nl-fixture","description":"local fixture package"}"#,
    )
    .expect("write fixture");
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "package.json".to_string();

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "describe package",
        Some(&route),
        &LoopState::new(1),
        "sanitized user request without machine hint block",
        None,
    )
    .expect("route-level contract hint should select deterministic two-step plan");

    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("stat_paths")
    );
    assert_eq!(plan.steps[1].skill, "fs_basic");
    assert_eq!(
        plan.steps[1].args.get("action").and_then(Value::as_str),
        Some("read_text_range")
    );
    assert!(plan.steps[1]
        .args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("package.json")));
}

#[test]
fn contract_hint_matrix_config_risk_uses_deterministic_guard_action() {
    let state = test_state_with_enabled_skills(&["config_basic", "config_edit"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "guard config",
        Some(&route),
        &LoopState::new(1),
        "sanitized request without hint block",
        None,
    )
    .expect("config risk contract should use deterministic guard action");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "config_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("guard_rustclaw_config")
    );
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn contract_hint_preferred_config_guard_uses_runtime_equivalent_action() {
    let state = test_state_with_enabled_skills(&["config_basic", "config_edit"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=config_guard\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "guard config",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("virtual config guard should map to runtime guard action");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "config_edit");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("guard_config")
    );
}

#[test]
fn contract_hint_file_paths_uses_machine_selector_extension() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=file_paths\nselector_extension=md\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list markdown paths",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("file path contract should use structured selector hints");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(
        plan.steps[0].args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local")
    );
    assert_eq!(
        plan.steps[0].args.get("extension").and_then(Value::as_str),
        Some("md")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("target_kind")
            .and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn contract_hint_recent_artifacts_uses_machine_sort_and_limit_selectors() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local/docs".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=recent_artifacts_judgment\nselector_limit=2\nselector_sort_by=mtime_desc\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list recent files and judge",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("recent artifact contract should use structured sort selectors");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("list_dir")
    );
    assert_eq!(
        plan.steps[0].args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("max_entries")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("files_only")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn contract_hint_file_names_uses_machine_file_kind_selector() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local/docs".to_string();
    let request =
            "[CONTRACT_TEST_HINT]\nsemantic_kind=file_names\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list file names",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("file name contract should use file-only selector hints");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("list_dir")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("files_only")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        plan.steps[0].args.get("dirs_only").is_none(),
        "file-only selector must not also request directories"
    );
}

#[test]
fn contract_hint_directory_entry_groups_find_entries_defaults_to_any_kind() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=directory_entry_groups\npreferred_action_ref=fs_basic.find_entries\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "group direct children by kind",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("directory entry grouping should preserve file and directory candidates");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("target_kind")
            .and_then(Value::as_str),
        Some("any")
    );
}

#[test]
fn contract_hint_archive_read_uses_matrix_preferred_action_without_nl_matching() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip|notes.txt".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=archive_read\ncandidate_wrong_action_ref=fs_basic.find_entries\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "read archive member",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("archive read contract should use matrix preferred archive action");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "archive_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("read")
    );
    assert_eq!(
        plan.steps[0].args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
    assert_eq!(
        plan.steps[0].args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn contract_hint_content_presence_uses_machine_query_and_case_selector() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=content_presence_check\nselector_query=release\nselector_case_insensitive=true\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check content presence",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("content presence contract should use structured query selector");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("grep_text")
    );
    assert_eq!(
        plan.steps[0].args.get("query").and_then(Value::as_str),
        Some("release")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("case_insensitive")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn contract_hint_preferred_doc_parse_uses_structured_parse_doc_action() {
    let state = test_state_with_enabled_skills(&["doc_parse"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=content_presence_check\npreferred_action_ref=doc_parse\nselector_query=release\nselector_case_insensitive=true\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check content presence using preferred parser",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("doc_parse preference should be planned without model fallback");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "doc_parse");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("parse_doc")
    );
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
    );
}

#[test]
fn contract_hint_hidden_entries_list_dir_includes_hidden_entries() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = ".".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=hidden_entries_check\npreferred_action_ref=fs_basic.list_dir\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check hidden entries",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("hidden entries contract should use deterministic inventory");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("list_dir")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("include_hidden")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn fs_basic_grep_text_case_sensitive_false_normalizes_to_case_insensitive() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "grep_text",
            "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "query": "release",
            "case_sensitive": false,
            "max_matches": 3
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "grep_text");
    assert_eq!(
        args.get("case_insensitive").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(3));
}

#[test]
fn fs_basic_read_text_range_range_tail_alias_becomes_mode_tail() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "logs/model_io.log",
            "range": "tail",
            "n": 4
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(4));
    assert!(args.get("range").is_none());
}

#[test]
fn fs_basic_read_text_range_negative_start_line_count_becomes_tail_count() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "logs/model_io.log",
            "start_line": -4,
            "line_count": 4
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(4));
    assert!(args.get("start_line").is_none());
    assert!(args.get("line_count").is_none());
}

#[test]
fn service_status_process_request_uses_process_basic_filter_plan() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "check clawd process",
        Some(&route),
        &loop_state,
        "check whether the local clawd process is present",
    )
    .expect("process status should use deterministic process_basic plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "process_basic", "ps");
    assert_eq!(args.get("filter").and_then(Value::as_str), Some("clawd"));
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(200));
}

#[test]
fn service_status_workspace_product_request_uses_health_check_plan() {
    let mut state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let tmp = TempDirGuard::new("rustclaw");
    let project_root = tmp.path.join("rustclaw");
    fs::create_dir_all(&project_root).expect("project root");
    state.skill_rt.workspace_root = project_root;
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "check local project service health",
        Some(&route),
        &loop_state,
        "检查本地 RustClaw 服务健康状态，简短输出状态",
    )
    .expect("workspace product status should use health_check plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "health_check");
            assert!(args.as_object().is_some_and(|obj| obj.is_empty()));
        }
        other => panic!("expected health_check action, got {other:?}"),
    }
}

#[test]
fn service_status_health_check_recipe_marker_uses_health_check_plan() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.route_reason = "execution_recipe_health_check_observation".to_string();
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "run a structured health observation",
        Some(&route),
        &loop_state,
        "run a structured health observation",
    )
    .expect("health check recipe marker should use health_check");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "health_check");
            assert!(args.as_object().is_some_and(|obj| obj.is_empty()));
        }
        other => panic!("expected health_check action, got {other:?}"),
    }
}

#[test]
fn scalar_service_status_uses_health_check_plan() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "return one runtime scalar",
        Some(&route),
        &loop_state,
        "current runtime scalar",
    )
    .expect("scalar service status should use health check");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "health_check");
            assert!(args.as_object().is_some_and(|obj| obj.is_empty()));
        }
        other => panic!("expected health_check action, got {other:?}"),
    }
}

#[test]
fn structural_contracts_are_not_blocked_by_literal_command_guard() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_hint = "package.json".to_string();

    assert!(structural_contract_deterministic_plan_overrides_literal_command_guard(Some(&route)));
}

#[test]
fn service_status_port_request_uses_process_basic_port_filter_plan() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "check local port",
        Some(&route),
        &loop_state,
        "show the process listening on local port 8787",
    )
    .expect("port status should use deterministic process_basic plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "process_basic", "port_list");
    assert_eq!(args.get("filter").and_then(Value::as_str), Some("8787"));
}

#[test]
fn service_status_process_ranking_count_is_not_port_filter() {
    let state = test_state_with_enabled_skills(&["process_basic", "system_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe process ranking",
        Some(&route),
        &loop_state,
        "看一下当前最占 CPU 的前 5 个进程，简短告诉我最值得注意的是哪个",
    );

    assert!(plan.is_none());
}

#[test]
fn service_status_without_process_target_uses_system_basic_info_plan() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe local runtime identity",
        Some(&route),
        &loop_state,
        "observe local runtime identity",
    )
    .expect("system status fallback should use system_basic info");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "system_basic", "info");
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn package_manager_dry_run_uses_commandish_answer_candidate() {
    let state = test_state_with_enabled_skills(&["package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.resolved_intent =
        "Show package preview\nanswer_candidate: command: sudo -n apt-get install -y ripgrep"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = package_manager_dry_run_deterministic_plan_result(
        &state,
        "dry-run package install",
        Some(&route),
        &loop_state,
        "ripgrep 설치는 하지 말고 dry-run 으로 어떤 명령이 될지만 알려줘.",
    )
    .expect("package manager dry-run should use deterministic plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "package_manager", "smart_install");
    assert_eq!(
        args.get("packages")
            .and_then(Value::as_array)
            .map(|packages| {
                packages
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["ripgrep"])
    );
    assert_eq!(args.get("dry_run").and_then(Value::as_bool), Some(true));
}

#[test]
fn package_manager_dry_run_falls_back_to_current_request_package_token() {
    let state = test_state_with_enabled_skills(&["package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.resolved_intent =
        "ripgrep install dry-run preview without executing installation".to_string();
    let loop_state = LoopState::new(1);

    let plan = package_manager_dry_run_deterministic_plan_result(
        &state,
        "dry-run package install",
        Some(&route),
        &loop_state,
        "ripgrep 설치는 하지 말고 dry-run 으로 어떤 명령이 될지만 알려줘.",
    )
    .expect("package manager dry-run should extract the safe current-request package token");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "package_manager", "smart_install");
    assert_eq!(
        args.get("packages")
            .and_then(Value::as_array)
            .map(|packages| {
                packages
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["ripgrep"])
    );
}

#[test]
fn archive_basic_unknown_readonly_action_normalizes_to_list_for_archive_contract() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = archive.to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "exists",
            "archive": archive,
            "entry": "nested/config.ini",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(Some(&route), actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_unknown_mutating_shape_does_not_normalize_to_list() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = archive.to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "make_archive",
            "source": "scripts/nl_tests/fixtures/device_local/docs",
            "archive": archive,
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(Some(&route), actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("make_archive")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn preferred_route_allows_more_specific_structured_tool_action() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "tmp/nl_archive_case.zip".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: json!({
                "action": "pack",
                "source": "scripts/skill_calls",
                "archive": "tmp/nl_archive_case.zip",
                "format": "zip"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{s2.text}}".to_string(),
        },
    ];

    assert!(super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        !super::actions_use_ad_hoc_command_without_route_preferred_skill(&state, &route, &actions)
    );
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn process_ps_run_cmd_rewrites_to_process_basic() {
    let state = test_state_with_enabled_skills(&["process_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "ps -eo pid,user,%cpu,cmd --sort=-%cpu | head -n 6"}),
    }];

    let rewritten = rewrite_process_ps_run_cmd_to_process_basic(
        &state,
        "看一下当前最占 CPU 的前 5 个进程",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "process_basic", "ps");
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(5));
}

#[test]
fn process_ps_run_cmd_preserves_explicit_literal_command() {
    let state = test_state_with_enabled_skills(&["process_basic", "run_cmd"]);
    let command = "ps -eo pid,user,%cpu,cmd --sort=-%cpu | head -n 6";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let rewritten = rewrite_process_ps_run_cmd_to_process_basic(
        &state,
        &format!("执行 {command}"),
        None,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn process_ps_run_cmd_preserves_literal_flag() {
    let state = test_state_with_enabled_skills(&["process_basic", "run_cmd"]);
    let command = "ps -eo pid,user,%cpu,cmd --sort=-%cpu | head -n 6";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": command,
            CLAWD_LITERAL_COMMAND_ARG: true,
        }),
    }];

    let rewritten = rewrite_process_ps_run_cmd_to_process_basic(
        &state,
        "看一下当前最占 CPU 的前 5 个进程",
        None,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn docker_ps_run_cmd_rewrites_to_docker_basic() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker ps -a"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("ps"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn docker_image_ls_run_cmd_rewrites_to_docker_basic_images() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker image ls"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("images"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn docker_version_run_cmd_rewrites_to_docker_basic_version() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker version"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("version"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn docker_readonly_preserves_explicit_literal_run_cmd() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker ps"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("docker ps")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn doc_parse_unsupported_transform_action_normalizes_to_parse_doc() {
    let state = test_state_with_enabled_skills(&["doc_parse"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: json!({
            "action": "summarize",
            "file_path": "/home/guagua/rustclaw/README.md",
            "max_chars": 8000
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&base_route_result()),
        &LoopState::default(),
        "Summarize README.md",
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "doc_parse");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("parse_doc")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some("/home/guagua/rustclaw/README.md")
            );
        }
        other => panic!("expected doc_parse action, got {other:?}"),
    }
}

#[test]
fn archive_auto_locator_plans_list_instead_of_text_read() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "Inspect the archive contents without unpacking it.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string();
    let loop_state = LoopState::new(1);

    assert!(
        scalar_content_auto_locator_observation_plan(
            Some(&route),
            Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
        )
        .is_none(),
        "archive files must not be planned as text reads"
    );

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "Inspect the archive",
        &state,
        Some(&route),
        &loop_state,
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
    )
    .expect("archive list plan");

    assert_eq!(plan.steps.len(), 3);
    let step = &plan.steps[0];
    assert_eq!(step.action_type, "call_skill");
    assert_eq!(step.skill, "archive_basic");
    assert_eq!(
        step.args.get("action").and_then(Value::as_str),
        Some("list")
    );
    assert_eq!(
        step.args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
}

#[test]
fn archive_read_contract_plans_direct_member_read() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "Read member notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt".to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
        "Read member notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    )
    .expect("archive read plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(
        args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_contract_ignores_non_archive_auto_locator() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = format!("Read notes.txt from {archive}");
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | notes.txt");
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw/tmp/contract_matrix_unpacked/notes.txt"),
        &format!("Read member notes.txt from {archive}"),
    )
    .expect("archive read plan should fall back to contract locator");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_contract_recovers_explicit_archive_path_when_locator_hint_is_empty() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let request = format!("读取 {archive} 里的 notes.txt 内容片段，并简短总结。");
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = request.clone();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint.clear();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw/tmp/contract_matrix_unpacked/notes.txt"),
        &request,
    )
    .expect("archive read plan should recover explicit archive path");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_contract_prefers_complete_request_path_over_basename_locator_hint() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let request = format!("读取 {archive} 里的 notes.txt 内容片段，并简短总结。");
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = request.clone();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = "test_bundle.zip | notes.txt".to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw/tmp/contract_matrix_unpacked/notes.txt"),
        &request,
    )
    .expect("archive read plan should restore full archive path");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_structural_member_target_plans_direct_read_without_semantic_label() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        format!("Read the notes.txt content from archive {archive} and output only it");
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_hint = archive.to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
        &format!("Read {archive} member notes.txt"),
    )
    .expect("archive read plan from structural member target");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_contract_rejects_unsafe_member_locator() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | ../secret.txt".to_string();
    let loop_state = LoopState::new(1);

    assert!(archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
        "Read member ../secret.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    )
    .is_none());
}

#[test]
fn transform_action_alias_and_sort_args_normalize_to_transform_data_ops() {
    let actions = vec![AgentAction::CallTool {
        tool: "transform".to_string(),
        args: json!({
            "action": "transform",
            "data": [
                {"name": "alpha", "score": 7},
                {"name": "beta", "score": 12}
            ],
            "sort_by": "score",
            "order": "desc",
            "output_format": "md_table"
        }),
    }];

    let normalized = normalize_transform_schema_aliases(actions);

    let args = expect_planned_call(&normalized[0], "transform", "transform_data");
    assert_eq!(
        args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
    let ops = args
        .get("ops")
        .and_then(Value::as_array)
        .expect("ops array");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].get("op").and_then(Value::as_str), Some("sort"));
    assert_eq!(ops[0].get("by").and_then(Value::as_str), Some("score"));
    assert_eq!(ops[0].get("order").and_then(Value::as_str), Some("desc"));
    assert!(args.get("sort_by").is_none());
}

#[test]
fn inline_json_transform_deterministic_plan_uses_current_payload() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"filter","where":{"field":"score","gte":7}}]}"#;
    let goal = r#"older context: {"action":"transform_data","data":[{"stale":true}],"ops":[{"op":"project","fields":["stale"]}]}"#;

    let plan =
        inline_json_transform_deterministic_plan_result(goal, &state, &loop_state, current, None)
            .expect("inline transform should produce deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.action_type, "call_skill");
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("action").and_then(Value::as_str),
        Some("transform_data")
    );
    assert_eq!(
        step.args
            .get("data")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("name"))
            .and_then(Value::as_str),
        Some("alpha")
    );
    let op = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(Value::as_object)
        .expect("normalized filter op");
    assert_eq!(op.get("field").and_then(Value::as_str), Some("score"));
    assert_eq!(op.get("cmp").and_then(Value::as_str), Some("gte"));
    assert_eq!(op.get("value").and_then(Value::as_i64), Some(7));
}

#[test]
fn inline_json_transform_derives_group_sum_from_structured_candidate() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"对这个 JSON 数组按 team 分组求 amount 总和，只输出 JSON：[{"team":"A","amount":3},{"team":"A","amount":4},{"team":"B","amount":2}]"#;
    let mut route = base_route_result();
    route.route_reason = "inline_json_transform_structured_execute".to_string();
    route.resolved_intent =
            "group inline records\nanswer_candidate: [{\"team\":\"A\",\"amount\":7},{\"team\":\"B\",\"amount\":2}]".to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    )
    .expect("inline transform should derive group sum");

    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.action_type, "call_skill");
    assert_eq!(step.skill, "transform");
    let op = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(Value::as_object)
        .expect("group op");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("group"));
    assert_eq!(
        op.get("by")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("team")
    );
    assert_eq!(
        op.get("aggregations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("field"))
            .and_then(Value::as_str),
        Some("amount")
    );
}

#[test]
fn contextual_inline_payload_derives_default_numeric_sort_table() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let mut route = base_route_result();
    route.route_reason = "inline_structured_payload_context_execute:test".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    )
    .expect("contextual inline payload should produce deterministic transform");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
    let op = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(Value::as_object)
        .expect("sort op");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("sort"));
    assert_eq!(op.get("by").and_then(Value::as_str), Some("score"));
    assert_eq!(op.get("order").and_then(Value::as_str), Some("desc"));
}

#[test]
fn repaired_inline_transform_contract_derives_default_numeric_sort_table() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"Sort this JSON array by score descending and output only a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;
    let mut route = base_route_result();
    route.route_reason = "inline_structured_transform_contract_repair".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    )
    .expect("repaired inline transform contract should produce deterministic transform");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
    let op = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(Value::as_object)
        .expect("sort op");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("sort"));
    assert_eq!(op.get("by").and_then(Value::as_str), Some("score"));
    assert_eq!(op.get("order").and_then(Value::as_str), Some("desc"));
}

#[test]
fn inline_json_transform_derives_single_object_rename_after_context_json() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"把这个 JSON 对象里的 old_name 改成 new_name，只输出 JSON：{"old_name":"alpha","count":2}"#;
    let goal = format!(
        r#"background example: {{"kind":"ask","payload":{{"text":"hello"}}}}

Structured inline transform request:
{current}"#
    );
    let mut route = base_route_result();
    route.route_reason = "inline_json_transform_structured_execute".to_string();
    route.resolved_intent = r#"rename inline object
answer_candidate: {"new_name":"alpha","count":2}"#
        .to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        &goal,
        &state,
        &loop_state,
        "",
        Some(&route),
    )
    .expect("context JSON should not steal inline object transform");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("result_shape").and_then(Value::as_str),
        Some("single_object")
    );
    assert!(step.args.get("data").is_some_and(Value::is_object));
}

#[test]
fn inline_json_transform_derives_single_object_rename_without_answer_candidate() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let req = r#"把这个 JSON 对象里的 old_name 改成 new_name，只输出 JSON：{"old_name":"alpha","count":2}"#;

    let plan = inline_json_transform_deterministic_plan_result(req, &state, &loop_state, req, None)
        .expect("single object rename should produce deterministic plan");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert!(step.args.get("data").is_some_and(Value::is_object));
    assert_eq!(
        step.args.get("result_shape").and_then(Value::as_str),
        Some("single_object")
    );
    assert_eq!(
        step.args
            .get("ops")
            .and_then(Value::as_array)
            .and_then(|ops| ops.first())
            .and_then(|op| op.get("op"))
            .and_then(Value::as_str),
        Some("rename")
    );
}

#[test]
fn inline_json_transform_derives_scalar_sum_after_context_json() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"计算这个 JSON 数组里 value 的总和，只输出数字：[ {"value": 4}, {"value": 6}, {"value": 5} ]"#;
    let goal = format!(
        r#"background example: {{"kind":"ask","payload":{{"text":"hello"}}}}

Structured inline transform request:
{current}"#
    );
    let mut route = base_route_result();
    route.route_reason = "inline_json_transform_structured_execute".to_string();
    route.resolved_intent = "sum inline records\nanswer_candidate: 15".to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        &goal,
        &state,
        &loop_state,
        "",
        Some(&route),
    )
    .expect("context JSON should not steal scalar aggregate transform");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("result_shape").and_then(Value::as_str),
        Some("scalar")
    );
    let op = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(Value::as_object)
        .expect("aggregate op");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("aggregate"));
}

#[test]
fn inline_json_transform_derives_count_from_scalar_count_contract() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2},{"x":3},{"x":4}]"#;
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    )
    .expect("inline scalar count should produce deterministic transform");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("result_shape").and_then(Value::as_str),
        Some("scalar")
    );
    let agg = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(|op| op.get("aggregations"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("count aggregation");
    assert_eq!(agg.get("op").and_then(Value::as_str), Some("count"));
}

#[test]
fn inline_csv_transform_derives_markdown_table_from_escaped_newlines() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = "把这个 CSV 转成 markdown 表格：name,score\\nalpha,7\\nbeta,9";
    let mut route = base_route_result();
    route.resolved_intent =
            "render inline records\nanswer_candidate: | name | score |\n|------|-------|\n| alpha | 7 |\n| beta | 9 |".to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    )
    .expect("escaped newline CSV should produce deterministic transform");

    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("csv_text").and_then(Value::as_str),
        Some("name,score\nalpha,7\nbeta,9")
    );
    assert_eq!(
        step.args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
}

#[test]
fn lightweight_prompt_mentions_archive_basic_for_archive_contracts() {
    let state = test_state_with_enabled_skills(&[
        "archive_basic",
        "docker_basic",
        "config_guard",
        "doc_parse",
        "transform",
        "browser_web",
    ])
    .with_prompt_layers_installed();
    let task = test_task();
    let quick_index = build_lightweight_skill_quick_index_text(&state, &task);
    let playbooks = build_lightweight_skill_playbooks_text(&state, &task);
    assert!(quick_index.contains("archive_basic"));
    assert!(playbooks.contains("archive_basic"));
    assert!(playbooks.contains("`pack`") || playbooks.contains("packing"));
    assert!(quick_index.contains("docker_basic"));
    assert!(playbooks.contains("docker_basic"));
    assert!(quick_index.contains("config_guard"));
    assert!(playbooks.contains("config_guard"));
    assert!(quick_index.contains("doc_parse"));
    assert!(playbooks.contains("doc_parse"));
    assert!(quick_index.contains("transform"));
    assert!(playbooks.contains("transform"));
    assert!(quick_index.contains("browser_web"));
    assert!(playbooks.contains("browser_web"));
}

#[test]
fn lightweight_prompt_includes_registry_planner_metadata() {
    let state = test_state_with_registry();
    let registry = state.get_skills_registry().expect("registry loaded");
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(HashSet::from([
            "archive_basic".to_string(),
            "service_control".to_string(),
        ])),
    });
    let state = state.with_prompt_layers_installed();
    let task = test_task();
    let quick_index = build_lightweight_skill_quick_index_text(&state, &task);
    let playbooks = build_lightweight_skill_playbooks_text(&state, &task);
    assert!(quick_index.contains("archive_basic"));
    assert!(quick_index.contains("planner_kind: tool"));
    assert!(quick_index.contains("semantic_tags: archive_list"));
    assert!(quick_index.contains("preferred_over_run_cmd: true"));
    assert!(quick_index.contains("validation_actions: list"));
    assert!(playbooks.contains("### archive_basic"));
    assert!(playbooks.contains("Registry metadata: planner_kind: tool"));
    assert!(playbooks.contains("semantic_tags: archive_list"));
    assert!(playbooks.contains("preferred_over_run_cmd: true"));
    assert!(playbooks.contains("validation_actions: list"));
    assert!(playbooks.contains("### service_control"));
    assert!(playbooks.contains("semantic_tags: service_status"));
}

#[test]
fn lightweight_skill_playbook_keeps_config_entry_points() {
    let prompt = r#"
## Capability Summary
- Converts audio to text.

## Config Entry Points
- Main STT config: `configs/audio.toml` -> `[audio_transcribe]`.
- Local provider uses `audio_transcribe.providers.custom`.

## Parameter Contract
- `path` is optional here.
"#;
    let compact = compact_skill_playbook_from_prompt("audio_transcribe", prompt);
    assert!(compact.contains("configs/audio.toml"));
    assert!(compact.contains("audio_transcribe.providers.custom"));
}

#[test]
fn lightweight_tool_spec_includes_route_task_contract() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;

    let spec = build_lightweight_tool_spec(Some(&route), None);

    assert!(spec.contains("task_contract"));
    assert!(spec.contains("intent_kind=planner_execute"));
    assert!(spec.contains("target_object=directory"));
    assert!(spec.contains("operation=list"));
    assert!(spec.contains("required_evidence_fields=candidates"));
    assert!(spec.contains("failure_policy=retry_with_alternatives"));
}

#[test]
fn planner_prompt_contract_guard_allows_present_compact_contract_block() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let contract_line =
        crate::contract_matrix::compact_prompt_line_for_route(&route).expect("contract line");
    let prompt = format!("System\n{contract_line}\nUser");

    ensure_required_contract_block_present(Some(&route), &prompt).expect("contract present");
}

#[test]
fn planner_prompt_contract_guard_fails_closed_when_compact_contract_block_missing() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    let err = ensure_required_contract_block_present(Some(&route), "System\nUser")
        .expect_err("missing contract block should fail closed");

    assert!(err.contains("prompt_budget_error"));
    assert!(err.contains("contract_line_hash="));
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_scalar_contract() {
    let mut route = base_route_result();
    route.route_reason = "llm_contract:generic_filename_scalar_extract".to_string();
    route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_generic_scalar_path_read() {
    let mut route = base_route_result();
    route.resolved_intent =
            "读取 /home/guagua/rustclaw/configs/config.toml 中的 tools.allow_sudo 配置项的值，并仅输出该值"
                .to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/configs/config.toml".to_string();
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_pwd_only_route() {
    let mut route = base_route_result();
    route.route_reason = "llm_contract:scalar_path_only".to_string();
    route.resolved_intent = "只输出当前工作目录的绝对路径，不要解释".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_content_evidence_reads() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.route_reason = "llm_contract:generic_filename_read_range".to_string();
    route.resolved_intent = "先读一下 README.md 前 4 行".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_keeps_open_planning_for_chat_wrapped_execution_or_later_rounds() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "比较这两个文件大小，然后一句话总结".to_string();
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "open_planning"
    );

    let mut scalar = base_route_result();
    scalar.route_reason = "llm_contract:generic_filename_scalar_extract".to_string();
    scalar.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
    scalar.output_contract.response_shape = OutputResponseShape::Scalar;
    scalar.output_contract.requires_content_evidence = true;
    scalar.output_contract.locator_kind = OutputLocatorKind::Filename;
    scalar.output_contract.locator_hint = "package.json".to_string();
    let mut round2 = LoopState::default();
    round2.round_no = 2;
    assert_eq!(
        classify_planning_prompt_class(Some(&scalar), &scalar.resolved_intent, &round2).as_str(),
        "open_planning"
    );
}

#[test]
fn planning_prompt_class_keeps_open_planning_for_current_workspace_drafting() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "Write a short RustClaw setup note for the current workspace project".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "rustclaw workspace".to_string();

    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "open_planning"
    );
}

#[test]
fn round1_prompt_spec_switches_to_lightweight_prompt_for_light_class() {
    assert_eq!(
        round1_prompt_spec_for_class(PlanningPromptClass::OpenPlanning),
        (
            "single_plan_execution_prompt",
            "prompts/single_plan_execution_prompt.md",
        )
    );
    assert_eq!(
        round1_prompt_spec_for_class(PlanningPromptClass::LightweightExecution),
        (
            "lightweight_execution_prompt",
            "prompts/lightweight_execution_prompt.md",
        )
    );
}

#[test]
fn lightweight_tool_spec_includes_contract_and_auto_locator() {
    let mut route = base_route_result();
    route.route_reason = "llm_contract:generic_explicit_path_scalar_extract".to_string();
    route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_hint = "UI/package.json".to_string();
    let rendered = build_lightweight_tool_spec(Some(&route), Some("/tmp/UI/package.json"));
    assert!(rendered.contains("planning_class=lightweight_execution"));
    assert!(rendered.contains("response_shape=scalar"));
    assert!(rendered.contains("locator_hint=UI/package.json"));
    assert!(rendered.contains("auto_locator_path=/tmp/UI/package.json"));
}

#[test]
fn rewrite_extract_field_field_alias_to_field_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/config.toml",
            "field": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("field_path").and_then(|value| value.as_str()),
                Some("tools.allow_sudo")
            );
            assert!(args.get("field").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_keeps_existing_field_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/config.toml",
            "field": "tools.allow_sudo",
            "field_path": "tools.allow_path_outside_workspace"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("field_path").and_then(|value| value.as_str()),
                Some("tools.allow_path_outside_workspace")
            );
            assert_eq!(
                args.get("field").and_then(|value| value.as_str()),
                Some("tools.allow_sudo")
            );
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_file_path_alias_to_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "file_path": "/tmp/config.toml",
            "field_path": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("/tmp/config.toml")
            );
            assert!(args.get("file_path").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_target_alias_to_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "target": "/tmp/config.toml",
            "field_path": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("/tmp/config.toml")
            );
            assert!(args.get("target").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn extract_field_rewrites_bare_manifest_to_shallow_candidate_with_field() {
    let root = TempDirGuard::new("structured_manifest_candidate");
    fs::write(
        root.path.join("package.json"),
        r#"{"dependencies":{"left-pad":"1.0.0"}}"#,
    )
    .expect("write root package");
    fs::create_dir_all(root.path.join("UI")).expect("create ui");
    fs::write(
        root.path.join("UI/package.json"),
        r#"{"name":"react-example"}"#,
    )
    .expect("write ui package");
    fs::create_dir_all(root.path.join("services/wa-web-bridge")).expect("create service");
    fs::write(
        root.path.join("services/wa-web-bridge/package.json"),
        r#"{"name":"wa-web-bridge"}"#,
    )
    .expect("write service package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_package = root.path.join("package.json");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_package.display().to_string(),
            "field_path": "name"
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "读取 package.json 里的 name 字段",
        Some(root_package.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root.path.join("UI/package.json").to_string_lossy().as_ref())
    );
}

#[test]
fn extract_field_rewrites_workspace_cargo_package_field_to_current_package_manifest() {
    let root = TempDirGuard::new("workspace_cargo_candidate");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/other", "crates/clawd"]
"#,
    )
    .expect("write workspace cargo");
    fs::create_dir_all(root.path.join("crates/other")).expect("create other");
    fs::write(
        root.path.join("crates/other/Cargo.toml"),
        r#"[package]
name = "other"
"#,
    )
    .expect("write other cargo");
    fs::create_dir_all(root.path.join("crates/clawd")).expect("create clawd");
    fs::write(
        root.path.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("write clawd cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.name"
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "读取 Cargo.toml 的 package.name",
        Some(root_cargo.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(
            root.path
                .join("crates/clawd/Cargo.toml")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn extract_field_rewrites_workspace_cargo_package_version_to_workspace_package_version() {
    let root = TempDirGuard::new("workspace_cargo_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.version"
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read workspace package version from Cargo.toml",
        Some(root_cargo.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_cargo.to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );
}

#[test]
fn config_basic_read_field_rewrites_workspace_cargo_package_version_to_workspace_package_version() {
    let root = TempDirGuard::new("config_workspace_cargo_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.version"
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read Cargo.toml version and answer as `version=<value>` only.",
        Some(root_cargo.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_cargo.to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );
}

fn should_force_plan_repair(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    should_force_actionable_plan_repair(&test_state(), route_result, loop_state, actions)
}

fn repair_reason(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Option<&[AgentAction]>,
) -> &'static str {
    plan_repair_reason(&test_state(), route_result, loop_state, actions)
}

fn route_result(
    ask_mode: AskMode,
    requires_content_evidence: bool,
    response_shape: OutputResponseShape,
) -> RouteResult {
    RouteResult {
        ask_mode,
        resolved_intent: "test".to_string(),
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
            response_shape,
            requires_content_evidence,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: Default::default(),
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn delivery_route_result() -> RouteResult {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::FileToken,
    );
    route.output_contract.delivery_required = true;
    route
}

#[test]
fn actionable_route_repairs_respond_only_plan_before_any_observation() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "final answer".to_string(),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn plain_act_path_action_rejects_readonly_file_plan_before_execution() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "document/nl_tool200/group_02/memo.txt".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt",
                "mode": "head",
                "n": 120
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "plain_act_file_action_requires_non_readonly_plan"
    );
}

#[test]
fn active_task_append_current_locator_uses_append_text_plan() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        attachment_processing_required: false,
        state_patch: Some(json!({
            "deictic_reference": {"target": "current_turn_locator"},
            "required_content_literals": ["beta"]
        })),
    };

    let plan = active_task_append_current_locator_deterministic_plan_result(
        "append to active file",
        Some(&route),
        &loop_state,
        Some(&analysis),
        Some("/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt"),
    )
    .expect("expected deterministic append plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("append_text")
    );
    assert_eq!(
        plan.steps[0].args.get("content").and_then(Value::as_str),
        Some("beta\n")
    );
    assert_eq!(plan.steps[1].action_type, "synthesize_answer");
}

#[test]
fn execute_route_without_content_evidence_rejects_doc_parse_only_file_plan() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        false,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "document/nl_tool200/group_02/memo.txt".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "doc_parse".to_string(),
            args: json!({
                "action": "parse_doc",
                "path": "/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt",
                "mode": "auto"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "execute_route_requires_non_readonly_file_plan"
    );
    assert!(!can_fallback_to_initial_plan_after_repair_failure(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn content_evidence_route_accepts_doc_parse_file_plan() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "doc_parse".to_string(),
            args: json!({
                "action": "parse_doc",
                "path": "/home/guagua/rustclaw/README.md",
                "mode": "auto"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn content_evidence_route_repairs_respond_only_plan_even_in_chat_mode() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "guessed answer".to_string(),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::direct_answer(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_evidence_route_repairs_synthesize_only_plan_before_any_observation() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "non_actionable_plan_for_current_route"
    );
}

#[test]
fn content_evidence_route_repairs_locator_only_observation_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "find_name",
            "pattern": "crates/clawd/src/prompt_utils.rs",
        }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn content_evidence_route_accepts_structured_listing_terminal_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "root": "/workspace",
                "target_kind": "file",
                "name_pattern": "README",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn existence_route_accepts_stat_paths_synthesized_metadata_evidence() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["README.md", "README.zh-CN.md", "Cargo.toml"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn existence_route_accepts_observation_only_stat_paths_for_runtime_finalizer() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "paths": ["/workspace/README.md"]
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "plan_missing_terminal_user_answer"
    );
}

#[test]
fn existence_route_accepts_observation_only_stat_paths_even_when_content_evidence_required() {
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "paths": ["/workspace/missing.txt"],
            "include_missing": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn generic_path_route_accepts_stat_paths_synthesized_metadata_evidence() {
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["scripts/nl_tests/fixtures/device_local/docs/missing.md"],
                "include_missing": true
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/missing.md".to_string();

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn directory_names_route_accepts_fs_basic_find_entries_evidence() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "root": "/workspace",
                "target_kind": "file",
                "ext_filter": "sh",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryNames;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn content_evidence_route_accepts_scoped_grep_observation_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "grep_text",
            "path": "crates/clawd/src/prompt_utils.rs",
            "query": "run_cmd",
        }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_presence_route_accepts_text_read_observation_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                "mode": "head",
                "n": 120
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;

    assert!(
        !should_force_plan_repair(Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        repair_reason(Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn workspace_synthesis_respond_only_plan_gets_default_evidence_actions() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    let actions = vec![AgentAction::Respond {
        content: "guessed release note".to_string(),
    }];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a short release note for RustClaw.",
        None,
        actions,
    );

    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("log")
    )));
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str())
                    == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    )));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn workspace_synthesis_plan_adds_missing_text_evidence_and_synthesizes_all_steps() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"tree_summary","path":"."}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action":"extract_fields",
                "path":"Cargo.toml",
                "field_paths":["workspace.package.version"]
            }),
        },
        AgentAction::Respond {
            content: "# Release\nSee README.md\n- guessed from Cargo.toml".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a short release note for RustClaw.",
        None,
        actions,
    );

    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("log")
    )));
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str())
                    == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    )));
    let synth_refs = normalized.iter().find_map(|action| match action {
        AgentAction::SynthesizeAnswer { evidence_refs } => Some(evidence_refs),
        _ => None,
    });
    assert_eq!(
        synth_refs,
        Some(&vec![
            "step_1".to_string(),
            "step_2".to_string(),
            "step_3".to_string(),
            "step_4".to_string(),
        ])
    );
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn workspace_discovery_only_plan_waits_for_text_evidence_before_synthesis() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "workspace_glance", "path": ".", "max_entries": 30}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "find_path", "name": "README.md", "target_kind": "file"}),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a deployment note for the current project.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
        )
    }));
}

#[test]
fn workspace_text_read_observation_can_append_synthesis() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action": "read_range", "path": "README.md", "mode": "head", "n": 40}),
    }];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a deployment note for the current project.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn workspace_default_evidence_does_not_expand_mixed_last_output_answer() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pwd"}),
        },
        AgentAction::Respond {
            content: "{{last_output}} 是当前工作目录，通常对应正在操作的项目根目录。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "执行 pwd，然后用一句话解释这个路径大概是什么",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. }
                if skill == "git_basic" || skill == "system_basic"
        )
    }));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string()]
                || evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn listing_grounded_workspace_synthesis_does_not_expand_default_text_evidence() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": ".", "names_only": true}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "List the current directory, then answer from that listing.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(!normalized.iter().any(|action| {
        matches!(action, AgentAction::CallSkill { skill, .. } if skill == "git_basic")
            || matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_range")
                        && args.get("path").and_then(Value::as_str) == Some("README.md")
            )
    }));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string()]
    ));
}

#[test]
fn workspace_default_evidence_does_not_expand_structured_count_answer() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "count_inventory", "path": "crates"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "count_inventory", "path": "crates/skills"}),
        },
        AgentAction::Respond {
            content: "{{s1.output}} | {{s2.output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "count two directories and explain the layout",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 4);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. } if skill == "git_basic"
        )
    }));
    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
}

#[test]
fn workspace_default_evidence_does_not_expand_single_structured_count_answer() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "count_inventory",
                "path": ".",
                "kind_filter": "file",
                "recursive": false,
                "include_hidden": false
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("count_entries")
    ));
    assert!(!normalized.iter().any(|action| {
        matches!(action, AgentAction::CallSkill { skill, .. } if skill == "git_basic")
            || matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_range")
            )
    }));
}

#[test]
fn structured_tool_output_placeholder_is_synthesized_before_respond() {
    let loop_state = LoopState::new(1);
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts"}),
        },
        AgentAction::Respond {
            content: "scripts has {{last_output}} entries".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "count scripts entries",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn fs_basic_append_text_aliases_text_to_content_before_verify() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "append_text",
            "path": "document/nl_tool200/group_02/memo.txt",
            "text": "beta"
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "append_text");
    assert_eq!(args.get("content").and_then(Value::as_str), Some("beta"));
    assert!(args.get("text").is_none());
}

#[test]
fn structured_scalar_compare_accepts_fs_basic_count_entries_pair() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "document"}),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn quantity_comparison_route_accepts_single_count_entries_scalar_plan() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts/nl_tests/fixtures/device_local/docs"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn unavailable_skill_plan_forces_repair() {
    let state = test_state_with_enabled_skills(&["run_cmd", "read_file"]);
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "disabled_writer".to_string(),
        args: json!({ "path": "out.txt" }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        false,
        OutputResponseShape::Free,
    );

    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "unavailable_skill_requires_replan"
    );
}

#[test]
fn preferred_registry_skill_route_forces_repair_but_can_fallback_to_safe_run_cmd() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl status clawd"}),
    }];

    assert!(super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        super::actions_use_ad_hoc_command_without_route_preferred_skill(&state, &route, &actions)
    );
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "preferred_skill_required_for_semantic_route"
    );
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn preferred_registry_skill_route_does_not_fallback_to_mutating_run_cmd() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl restart clawd"}),
    }];

    assert!(
        super::actions_use_ad_hoc_command_without_route_preferred_skill(&state, &route, &actions)
    );
    assert!(!can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn preferred_registry_skill_route_does_not_force_repair_from_structured_tool() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: json!({"action": "diagnose_runtime"}),
    }];

    assert!(super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        !super::actions_use_ad_hoc_command_without_route_preferred_skill(&state, &route, &actions)
    );
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn fs_basic_directory_names_route_forces_repair_from_run_cmd() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "find . -type f -name '*.sh' | xargs dirname | sort -u"}),
    }];

    assert!(super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        super::actions_use_ad_hoc_command_without_route_preferred_skill(&state, &route, &actions)
    );
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "preferred_skill_required_for_semantic_route"
    );
}

#[test]
fn explicit_literal_run_cmd_marker_skips_preferred_skill_repair() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "sqlite3 data/db-basic-contract.sqlite '.tables'",
            super::super::CLAWD_LITERAL_COMMAND_ARG: true
        }),
    }];

    assert!(super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        !super::actions_use_ad_hoc_command_without_route_preferred_skill(&state, &route, &actions)
    );
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn explicit_literal_existing_run_cmd_is_marked_before_repair_checks() {
    let mut state = test_state_with_registry();
    state.policy.command_intent.execute_prefixes = vec!["执行命令 ".to_string()];
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "sqlite3 data/db-basic-contract.sqlite '.tables'"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "执行 sqlite3 命令查询 data/db-basic-contract.sqlite 数据库中的所有表名，并返回结果。",
        Some("执行命令 sqlite3 data/db-basic-contract.sqlite \".tables\"，告诉我结果。"),
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool),
                Some(true)
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn explicit_literal_scalar_route_marks_failure_repairable() {
    let mut state = test_state_with_registry();
    state.policy.command_intent.execute_prefixes = vec!["执行 ".to_string()];
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "missing_probe --version"}),
    }];

    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行 missing_probe --version；如果该命令不存在，则执行 which bash，并只返回 bash 的路径。",
            Some(
                "执行 missing_probe --version；如果该命令不存在，则执行 which bash，并只返回 bash 的路径。",
            ),
            None,
            actions,
        );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                args.get(super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG)
                    .and_then(Value::as_bool),
                Some(true)
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn file_paths_route_marks_missing_target_repairable() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({"path": "plan/missing.md"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "read missing, then find a related file",
        Some("read missing, then find a related file"),
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get(super::super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG)
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn raw_command_output_route_does_not_force_preferred_skill_repair() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl status clawd"}),
    }];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn repair_failure_does_not_fallback_to_unavailable_skill_plan() {
    let state = test_state_with_enabled_skills(&["run_cmd", "read_file"]);
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "disabled_reader".to_string(),
        args: json!({ "path": "README.md" }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        false,
        OutputResponseShape::Free,
    );

    assert!(!can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn actionable_route_allows_respond_only_after_observation_exists() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "final answer".to_string(),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_evidence_route_keeps_observation_only_plan_for_observed_finalize() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: serde_json::json!({ "path": "README.md" }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn lightweight_act_route_keeps_observation_only_plan_without_repair() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "read_range",
            "path": "/tmp/device_local/logs/model_io.log",
            "mode": "tail",
            "n": 4
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = "读取 /tmp/device_local/logs/model_io.log 最后 4 行".to_string();
    route.output_contract.locator_hint = "/tmp/device_local/logs/model_io.log".to_string();
    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn lightweight_route_rejects_unavailable_followup_skill() {
    let state = test_state_with_enabled_skills(&["read_file"]);
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "README.md" }),
        },
        AgentAction::CallSkill {
            skill: "formatter".to_string(),
            args: serde_json::json!({ "text": "用一句话总结 {{last_output}}" }),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.route_reason = "llm_contract:generic_filename_single_read".to_string();
    route.resolved_intent = "看一下 README.md，然后一句话说它主要讲了什么".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "unavailable_skill_requires_replan"
    );
}

#[test]
fn clarify_followup_tail_request_does_not_rewrite_single_read_file_from_text() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target/content: scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({
                "path": "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "scripts/nl_tests/fixtures/device_local/logs/model_io.log",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some("scripts/nl_tests/fixtures/device_local/logs/model_io.log")
    );
}

#[test]
fn non_range_single_read_keeps_read_file_plan() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent =
        "看看 scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({
            "path": "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "scripts/nl_tests/fixtures/device_local/logs/model_io.log",
        None,
        actions,
    );

    assert!(
        planned_call_is(&normalized[0], "fs_basic", "read_text_range"),
        "normalized[0]={:?}",
        normalized[0]
    );
}

#[test]
fn single_target_read_file_prefers_auto_locator_file_over_stale_existing_path() {
    let state = test_state();
    let root = TempDirGuard::new("single_target_read_file");
    let stale = root.path.join("stale.log");
    let current = root.path.join("clawd.log");
    fs::write(&stale, "stale\n").expect("write stale file");
    fs::write(&current, "fresh\n").expect("write current file");
    let stale_path = stale.display().to_string();
    let current_path = current.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = format!("读取 {} 的内容", current_path);
    route.output_contract.locator_hint = current_path.clone();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": stale_path }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "第二个的内容",
        Some(current_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
}

#[test]
fn single_target_read_range_prefers_auto_locator_file_over_stale_existing_path() {
    let state = test_state();
    let root = TempDirGuard::new("single_target_read_range");
    let stale = root.path.join("hello_from_manual_test.sh");
    let current = root.path.join("clawd.log");
    fs::write(&stale, "#!/bin/bash\necho stale\n").expect("write stale file");
    fs::write(&current, "line1\nline2\n").expect("write current file");
    let stale_path = stale.display().to_string();
    let current_path = current.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = format!("查看 {} 最后 2 行", current_path);
    route.output_contract.locator_hint = current_path.clone();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": stale_path,
                "mode": "tail",
                "n": 2
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "第二个的最后 2 行",
        Some(current_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
}

#[test]
fn auto_locator_file_does_not_collapse_multi_read_plan() {
    let state = test_state();
    let root = TempDirGuard::new("multi_read_preserve");
    let alpha = root.path.join("alpha.log");
    let beta = root.path.join("beta.log");
    fs::write(&alpha, "alpha\n").expect("write alpha");
    fs::write(&beta, "beta\n").expect("write beta");
    let alpha_path = alpha.display().to_string();
    let beta_path = beta.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = "对比两个文件".to_string();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": alpha_path.clone() }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": beta_path.clone() }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "对比 alpha 和 beta",
        Some(beta_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(alpha_path.as_str())
    );
    let args = expect_planned_call(&normalized[1], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(beta_path.as_str())
    );
}

#[test]
fn content_evidence_route_keeps_terminal_discussion_followup_for_planned_synthesis() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn content_evidence_route_keeps_terminal_synthesize_followup_for_planned_synthesis() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn content_evidence_route_keeps_multi_evidence_synthesize_followup() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "service_notes.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "README.md" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 4);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::CallSkill { skill, .. } if skill == "read_file"
    ));
    assert!(matches!(
        &kept[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["s1".to_string(), "s2".to_string()]
    ));
    assert!(matches!(
        &kept[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn scalar_path_observation_strips_guessed_terminal_respond() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["/workspace/stem_unique/abcd"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "/workspace/stem_unique/abcd".to_string(),
        },
    ];

    let kept =
        strip_terminal_discussion_for_scalar_path_observation(Some(&route), &loop_state, actions);
    assert_eq!(kept.len(), 1);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
}

#[test]
fn scalar_path_observation_does_not_strip_after_tool_output_started() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["/workspace/stem_unique/abcd"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "/workspace/stem_unique/abcd".to_string(),
        },
    ];

    let kept = strip_terminal_discussion_for_scalar_path_observation(
        Some(&route),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::Respond { content } if content == "/workspace/stem_unique/abcd"
    ));
}

#[test]
fn system_basic_compare_paths_targets_alias_sets_left_and_right_paths() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "compare_paths",
            "targets": ["README.md", "AGENTS.md"],
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("compare_paths")
            );
            assert_eq!(
                args.get("left_path").and_then(|value| value.as_str()),
                Some("README.md")
            );
            assert_eq!(
                args.get("right_path").and_then(|value| value.as_str()),
                Some("AGENTS.md")
            );
        }
        other => panic!("expected system_basic compare_paths action, got {other:?}"),
    }
}

#[test]
fn system_basic_compare_paths_numbered_alias_sets_left_and_right_paths() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "compare_paths",
            "path1": "Cargo.lock",
            "path2": "Cargo.toml",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("left_path").and_then(|value| value.as_str()),
                Some("Cargo.lock")
            );
            assert_eq!(
                args.get("right_path").and_then(|value| value.as_str()),
                Some("Cargo.toml")
            );
            assert!(args.get("path1").is_none());
            assert!(args.get("path2").is_none());
        }
        other => panic!("expected system_basic compare_paths action, got {other:?}"),
    }
}

#[test]
fn system_basic_path_batch_facts_path_alias_becomes_paths_array() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "path_batch_facts",
            "path": "Cargo.toml",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(args.get("paths"), Some(&json!(["Cargo.toml"])));
            assert!(args.get("path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn system_basic_path_batch_facts_path_list_alias_becomes_paths() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "path_batch_facts",
            "path_list": ["Cargo.toml", "Cargo.lock"],
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(
                args.get("paths"),
                Some(&json!(["Cargo.toml", "Cargo.lock"]))
            );
            assert!(args.get("path_list").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn directory_read_range_after_inventory_is_stripped() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/docs",
                "sort_by": "mtime_desc",
                "max_entries": 2,
                "names_only": false,
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/docs/",
                "mode": "head",
                "n": 50,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "last_output".to_string(),
                "s1".to_string(),
                "s2".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = strip_directory_read_range_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        )
    }));
    match &normalized[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(
                evidence_refs,
                &vec!["last_output".to_string(), "s1".to_string()]
            );
        }
        other => panic!("expected synthesize_answer after inventory, got {other:?}"),
    }
}

#[test]
fn child_file_read_range_after_inventory_is_kept() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "/workspace/docs"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/docs/README.md",
                "mode": "head",
                "n": 20,
            }),
        },
    ];

    let normalized = strip_directory_read_range_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_range")
    ));
}

#[test]
fn unresolved_template_reads_after_inventory_are_stripped() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/docs",
                "sort_by": "mtime_desc",
                "max_entries": 2,
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "{{s1.entry0_path}}"}),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "{{s1.entry1_path}}"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string(), "s3".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. } if skill == "read_file"
        )
    }));
    match &normalized[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(evidence_refs, &vec!["s1".to_string()]);
        }
        other => panic!("expected synthesize_answer after inventory, got {other:?}"),
    }
}

#[test]
fn unresolved_template_reads_after_fs_search_are_stripped() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({
                "action": "find_name",
                "pattern": "missing.txt",
                "target_kind": "file",
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "{{last_output}}",
                "mode": "head",
                "n": 3,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        )
    }));
    match &normalized[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(evidence_refs, &vec!["step_1".to_string()]);
        }
        other => panic!("expected synthesize_answer after fs_search, got {other:?}"),
    }
}

#[test]
fn indexed_last_output_reads_after_inventory_are_kept() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/logs",
                "sort_by": "mtime_desc",
                "max_entries": 2,
                "names_only": true,
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/logs/{{last_output.0}}",
                "mode": "head",
                "n": 40,
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/logs/{{ last_output[1] }}",
                "mode": "head",
                "n": 40,
            }),
        },
    ];

    let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_range")
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_range")
    ));
}

#[test]
fn scalar_path_auto_locator_file_builds_observation_plan() {
    let root = TempDirGuard::new("scalar_auto_locator");
    let report = root.path.join("Report.MD");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let route = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "只输出匹配文件路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    let actions =
        scalar_path_auto_locator_observation_plan(Some(&route), Some(&report_path)).unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([report_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn scalar_path_locator_hint_file_builds_observation_plan_before_auto_locator() {
    let root = TempDirGuard::new("scalar_locator_hint");
    let selected = root.path.join("selected.md");
    let other = root.path.join("other.md");
    fs::write(&selected, "selected").expect("write selected");
    fs::write(&other, "other").expect("write other");
    let selected_path = selected.display().to_string();
    let other_path = other.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = selected_path.clone();

    let actions =
        scalar_path_auto_locator_observation_plan(Some(&route), Some(&other_path)).unwrap();

    let args = expect_planned_call(&actions[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!([selected_path])));
}

#[test]
fn scalar_path_auto_locator_deterministic_plan_uses_structural_locator() {
    let root = TempDirGuard::new("scalar_auto_locator_deterministic_plan");
    let report = root.path.join("my_abcd.txt");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "my_abcd.txt".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = scalar_path_auto_locator_deterministic_plan_result(
        "return the structurally resolved path",
        Some(&route),
        &loop_state,
        Some(&report_path),
    )
    .expect("fast plan should be available");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    assert!(plan.raw_plan_text.contains("stat_paths"));
    assert!(plan.raw_plan_text.contains(&report_path));
}

#[test]
fn file_facts_auto_locator_builds_stat_paths_synthesis_plan() {
    let root = TempDirGuard::new("file_facts_auto_locator");
    let report = root.path.join("README.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&report_path)).unwrap();

    assert_eq!(actions.len(), 3);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([report_path])));
            assert_eq!(
                args.get("fields"),
                Some(&json!(["exists", "kind", "size", "modified"]))
            );
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
    assert!(matches!(actions[1], AgentAction::SynthesizeAnswer { .. }));
    assert!(matches!(
        &actions[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn file_facts_auto_locator_does_not_override_content_semantic() {
    let root = TempDirGuard::new("file_facts_content_semantic");
    let report = root.path.join("README.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();

    assert!(file_facts_auto_locator_observation_plan(Some(&route), Some(&report_path)).is_none());
}

#[test]
fn file_facts_auto_locator_accepts_single_file_metadata_mislabeled_as_quantity_comparison() {
    let root = TempDirGuard::new("file_facts_quantity_comparison");
    let report = root.path.join("README.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&report_path)).unwrap();

    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths") == Some(&json!([report_path]))
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn file_facts_auto_locator_accepts_single_directory_metadata_quantity_comparison() {
    let root = TempDirGuard::new("directory_facts_quantity_comparison");
    let target = root.path.join("target");
    fs::create_dir_all(&target).expect("create target dir");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 4);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths") == Some(&json!([target_path.clone()]))
    ));
    assert!(matches!(
        &actions[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("count_entries")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
                && args.get("recursive").and_then(Value::as_bool) == Some(true)
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn strict_quantity_directory_target_uses_ranked_size_inventory() {
    let root = TempDirGuard::new("directory_facts_quantity_top_files");
    let target = root.path.join("logs");
    fs::create_dir_all(&target).expect("create target dir");
    fs::write(target.join("small.log"), "a").expect("write small");
    fs::write(target.join("large.log"), "abcdef").expect("write large");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "List the largest 3 files in the selected directory by size.".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 3);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("list_dir")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
                && args.get("files_only").and_then(Value::as_bool) == Some(true)
                && args.get("sort_by").and_then(Value::as_str) == Some("size_desc")
                && args.get("max_entries").and_then(Value::as_u64) == Some(3)
    ));
}

#[test]
fn strict_quantity_directory_without_selector_uses_path_metadata_plan() {
    let root = TempDirGuard::new("directory_facts_quantity_metadata");
    let target = root.path.join("target");
    fs::create_dir_all(&target).expect("create target dir");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 4);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths") == Some(&json!([target_path.clone()]))
    ));
    assert!(matches!(
        &actions[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("count_entries")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn free_quantity_directory_target_uses_broader_ranked_inventory() {
    let root = TempDirGuard::new("directory_facts_quantity_free_inventory");
    let target = root.path.join("schemas");
    fs::create_dir_all(&target).expect("create target dir");
    fs::write(target.join("small.json"), "{}").expect("write small");
    fs::write(target.join("large.json"), "{\"title\":\"larger\"}").expect("write large");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent =
        "List the selected directory's JSON files and describe the largest one.".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 3);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("list_dir")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
                && args.get("files_only").and_then(Value::as_bool) == Some(true)
                && args.get("sort_by").and_then(Value::as_str) == Some("size_desc")
                && args.get("max_entries").and_then(Value::as_u64) == Some(50)
    ));
}

#[test]
fn file_facts_auto_locator_deterministic_plan_resolves_current_workspace_quantity_target() {
    let root = TempDirGuard::new("directory_facts_quantity_current_workspace");
    let target = root.path.join("target");
    fs::create_dir_all(&target).expect("create target dir");
    let target_path = target.canonicalize().expect("canonical target");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "target".to_string();

    let plan = file_facts_auto_locator_deterministic_plan_result(
        &state,
        "inspect target metadata",
        Some(&route),
        &LoopState::new(1),
        "inspect target metadata",
        None,
        None,
    )
    .expect("deterministic file facts plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 4);
    assert!(plan.raw_plan_text.contains("stat_paths"));
    assert!(plan.raw_plan_text.contains("count_entries"));
    assert!(plan
        .raw_plan_text
        .contains(&target_path.display().to_string()));
}

#[test]
fn quantity_compare_pair_locator_uses_compare_paths_without_planner_guessing() {
    let root = TempDirGuard::new("quantity_compare_pair_locator");
    fs::write(root.path.join("Cargo.lock"), "abcdef").expect("write lock");
    fs::write(root.path.join("Cargo.toml"), "abc").expect("write toml");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock | Cargo.toml".to_string();

    let plan = quantity_compare_pair_locator_deterministic_plan_result(
        &state,
        "compare two path metadata targets",
        Some(&route),
        &LoopState::new(1),
    )
    .expect("deterministic quantity comparison pair plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "compare_paths");
    assert!(args
        .get("left_path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("Cargo.lock")));
    assert!(args
        .get("right_path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("Cargo.toml")));
}

#[test]
fn quantity_directory_inventory_injects_structural_extension_filter() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "List the selected directory's .json files and identify the largest file.".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "prompts/schemas".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "prompts/schemas",
            "files_only": true,
            "names_only": false,
            "sort_by": "size_desc",
            "max_entries": 5,
        }),
    }];

    let rewritten =
        inject_structural_extension_filter_for_directory_inventory(Some(&route), actions);

    let args = expect_planned_call(&rewritten[0], "fs_basic", "list_dir");
    assert_eq!(args.get("ext_filter"), Some(&json!(["json"])));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(1000));
}

#[test]
fn single_path_metadata_facts_do_not_satisfy_multi_target_quantity_comparison() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["README.md"],
                "fields": ["kind", "size_bytes"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn explicit_command_deterministic_plan_preserves_pipeline_literal() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行命令".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "运行命令 `printf rustclaw | wc -c`，只输出数字";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command",
        Some(&route),
        &loop_state,
        request,
    )
    .expect("explicit command should produce run_cmd plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("printf rustclaw | wc -c")
    );
    assert_eq!(
        plan.steps[0].args.get(CLAWD_LITERAL_COMMAND_ARG),
        Some(&json!(true))
    );
}

#[test]
fn explicit_configured_command_with_followup_skips_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request =
        "Run pwd, then run definitely_missing_command_rustclaw_english_67890, then summarize.";

    assert_eq!(
        super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd")
    );
    assert!(explicit_command_deterministic_plan_result(
        &state,
        "run compound explicit commands",
        Some(&route),
        &loop_state,
        request,
    )
    .is_none());
}

#[test]
fn prefixed_path_command_with_structural_args_before_freeform_tail_is_detected() {
    let root = TempDirGuard::new("prefixed_path_command_tail");
    fs::write(root.path.join("uname"), "").expect("write command marker");

    assert_eq!(
        super::path_command_segment_before_freeform_tail_with_path_env(
            "uname -a and tell me the result",
            Some(root.path.as_os_str()),
        ),
        Some("uname -a")
    );
    assert!(
        super::path_command_segment_before_freeform_tail_with_path_env(
            "uname and tell me the result",
            Some(root.path.as_os_str()),
        )
        .is_none()
    );
}

#[test]
fn explicit_configured_path_command_with_structural_args_is_preserved() {
    let path_env = std::env::var_os("PATH");
    if !super::command_token_resolves_in_path("uname", path_env.as_deref()) {
        return;
    }
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["please run ".to_string()];

    assert_eq!(
        super::explicit_command_segment(
            &state.policy.command_intent,
            "please run uname -a and tell me the result",
        )
        .as_deref(),
        Some("uname -a")
    );
}

#[test]
fn explicit_configured_command_without_followup_keeps_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run pwd,";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command",
        Some(&route),
        &loop_state,
        request,
    )
    .expect("single configured command should keep deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd")
    );
}

#[test]
fn explicit_configured_command_inside_clause_is_detected() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["请执行".to_string(), "执行".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "先别管方案，请执行 pwd，只输出命令结果。";

    assert_eq!(
        super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd")
    );
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command",
        Some(&route),
        &loop_state,
        request,
    )
    .expect("configured command in a later clause should produce run_cmd plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd")
    );
}

#[test]
fn embedded_standalone_command_with_structural_args_keeps_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "运行 pwd -P，只返回物理工作目录路径";

    assert_eq!(
        super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd -P")
    );
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command",
        Some(&route),
        &loop_state,
        request,
    )
    .expect("embedded standalone command should produce run_cmd plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd -P")
    );
}

#[test]
fn embedded_standalone_command_sequence_uses_configured_command_tokens() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "请依次执行 pwd 和 whoami，直接输出两个命令结果，每个结果一行，不要总结";

    assert_eq!(
        super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd; whoami")
    );
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command sequence",
        Some(&route),
        &loop_state,
        request,
    )
    .expect("configured command sequence should produce one run_cmd plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd; whoami")
    );
}

#[test]
fn leading_shellish_command_sequence_uses_path_commands() {
    let root = TempDirGuard::new("leading_shellish_command_sequence");
    for command in ["pwd", "whoami", "hostname"] {
        fs::write(root.path.join(command), "").expect("write command marker");
    }

    assert_eq!(
        super::leading_shellish_command_sequence_segment_with_path_env(
            "pwd whoami hostname 三个结果每个一行",
            Some(root.path.as_os_str()),
        )
        .as_deref(),
        Some("pwd; whoami; hostname")
    );
}

#[test]
fn leading_shellish_command_sequence_rejects_plain_status_words() {
    let root = TempDirGuard::new("leading_shellish_command_sequence_reject_status");
    for command in ["pwd", "whoami", "hostname"] {
        fs::write(root.path.join(command), "").expect("write command marker");
    }

    assert!(
        super::leading_shellish_command_sequence_segment_with_path_env(
            "show status",
            Some(root.path.as_os_str()),
        )
        .is_none()
    );
}

#[test]
fn leading_shellish_command_sequence_rejects_command_with_argument_shape() {
    let root = TempDirGuard::new("leading_shellish_command_sequence_reject_arg");
    fs::write(root.path.join("ls"), "").expect("write command marker");

    assert!(
        super::leading_shellish_command_sequence_segment_with_path_env(
            "ls scripts 结果每行一个",
            Some(root.path.as_os_str()),
        )
        .is_none()
    );
}

#[test]
fn explicit_prefixed_shellish_code_span_keeps_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run `pwd && ls Cargo.toml`.";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit shell code span",
        Some(&route),
        &loop_state,
        request,
    )
    .expect("shellish code span should keep deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd && ls Cargo.toml")
    );
}

#[test]
fn existence_with_path_filename_deterministic_plan_uses_name_search() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "start-all-bin.sh".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "find the file in the current repository",
        Some(&route),
        &loop_state,
        None,
        "find start-all-bin.sh in the current repository",
    )
    .expect("existence-with-path filename route should use a bounded search");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("start-all-bin.sh")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_multi_file_targets_deterministic_plan_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "检查 README.md、AGENTS.md、Cargo.toml 是否都存在，只用一行回答每个文件的存在状态"
            .to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check several explicit files",
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw"),
        "检查 README.md、AGENTS.md、Cargo.toml 是否都存在，只用一行回答每个文件的存在状态",
    )
    .expect("multi-file existence route should use path facts");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(
                args.get("paths"),
                Some(&json!(["README.md", "AGENTS.md", "Cargo.toml"]))
            );
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_multi_file_targets_preserve_relative_path_segments() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = "Check existence and type of two fixture paths".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let user_text = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check several explicit relative fixture paths",
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw"),
        user_text,
    )
    .expect("multi-path existence route should preserve explicit relative paths");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(
                args.get("paths"),
                Some(&json!([
                    "scripts/nl_tests/fixtures/device_local/package.json",
                    "scripts/nl_tests/fixtures/device_local/nope.json"
                ]))
            );
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_current_workspace_single_file_target_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "Check if README.md exists in the current directory and answer with the path".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check one explicit file in current workspace",
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw"),
        "Check if README.md exists in the current directory and answer with the path",
    )
    .expect("single-file current-workspace existence route should use path facts");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!(["README.md"])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_path_deterministic_plan_uses_path_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check exact path existence",
        Some(&route),
        &loop_state,
        Some("/tmp/Cargo.lock"),
        "check exact path Cargo.lock existence",
    )
    .expect("existence-with-path path route should use path facts");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!(["/tmp/Cargo.lock"])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn archive_entry_existence_uses_archive_list_instead_of_archive_stat() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        format!("Check whether archive member nested/config.ini is present in {archive}.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let stat_plan = existence_with_path_locator_deterministic_plan_result(
        "check archive member existence",
        Some(&route),
        &loop_state,
        Some(archive),
        "nested/config.ini in scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    );
    assert!(
        stat_plan.is_none(),
        "archive member checks must not be answered by statting only the archive file"
    );

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "check archive member existence",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
    )
    .expect("archive member existence should inspect archive entries");

    assert_eq!(plan.steps.len(), 3);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
        }
        other => panic!("expected archive_basic list action, got {other:?}"),
    }
}

#[test]
fn archive_entry_existence_scalar_shape_uses_archive_list() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.resolved_intent =
        format!("Check whether archive member notes.txt is present in {archive}.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let stat_plan = existence_with_path_locator_deterministic_plan_result(
            "check archive member scalar existence",
            Some(&route),
            &loop_state,
            Some(archive),
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 에 notes.txt 가 있는지만 말해. 압축 풀지 마.",
        );
    assert!(
        stat_plan.is_none(),
        "archive member scalar checks must not stat only the archive file"
    );

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "check archive member scalar existence",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
    )
    .expect("archive member scalar existence should inspect archive entries");

    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
        }
        other => panic!("expected archive_basic list action, got {other:?}"),
    }
}

#[test]
fn archive_file_existence_without_member_target_still_stats_archive() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = format!("Check whether {archive} exists.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check archive file existence",
        Some(&route),
        &loop_state,
        Some(archive),
        "Check whether scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip exists.",
    )
    .expect("plain archive file existence should use path facts");

    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([archive])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_directory_locator_with_file_target_uses_find_path() {
    let root = TempDirGuard::new("existence_dir_locator_file_target");
    fs::create_dir_all(root.path.join("case_only")).expect("mkdir");
    let directory = root.path.join("case_only");
    let directory_path = directory.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "Locate report.md within the specified directory and output only its full path."
            .to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = directory_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "find a file inside a resolved directory",
        Some(&route),
        &loop_state,
        Some(&directory_path),
        "Locate report.md within the specified directory and output only its full path.",
    )
    .expect("directory locator with file target should use find_path");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("root").and_then(Value::as_str),
                Some(directory_path.as_str())
            );
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("report.md")
            );
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_directory_auto_locator_does_not_parse_history_entries_as_targets() {
    let root = TempDirGuard::new("existence_dir_locator_history_entries");
    fs::create_dir_all(root.path.join("configs")).expect("mkdir configs");
    let directory_path = root.path.join("configs").display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = "Current task:\n先列出 configs 目录下前 5 个条目名称\n\nMost recent generated output:\nagent_guard.toml\naudio.toml\nbrowser_web_wait_map.json\nchannel_commands.toml\nchannels\n\nNew user instruction:\n看最后一个的基本信息，只回答路径和类型".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = directory_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "follow up on the active ordered list",
        Some(&route),
        &loop_state,
        Some(&directory_path),
        "看最后一个的基本信息，只回答路径和类型",
    );

    assert!(
            plan.is_none(),
            "directory auto-locator followups without current-turn locator surface should stay with planner/anchor resolution"
        );
}

#[test]
fn file_paths_current_workspace_deterministic_plan_uses_name_search() {
    let root = TempDirGuard::new("file_paths_deterministic_plan");
    let script = root.path.join("start-all-bin.sh");
    fs::write(&script, "#!/usr/bin/env bash\n").expect("write script");
    let script_path = script.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "start-all-bin.sh".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = file_paths_locator_deterministic_plan_result(
        "find a matching file and return its relative path",
        Some(&route),
        &loop_state,
        Some(&script_path),
    )
    .expect("file-path route should use a bounded name search");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("start-all-bin.sh")
            );
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn file_paths_path_like_locator_hint_uses_parent_search_scope() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "case_only/report.md".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = file_paths_locator_deterministic_plan_result(
        "find path-like locator under its parent scope",
        Some(&route),
        &loop_state,
        None,
    )
    .expect("path-like file locator should preserve parent scope");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(args.get("root").and_then(Value::as_str), Some("case_only"));
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("report.md")
            );
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn file_paths_deterministic_plan_does_not_treat_directory_locator_as_filename_pattern() {
    let root = TempDirGuard::new("file_paths_directory_locator");
    fs::write(root.path.join("lib.rs"), "fn direct_answer_gate() {}\n").expect("write rust");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = file_paths_locator_deterministic_plan_result(
        "search files under a directory",
        Some(&route),
        &loop_state,
        Some(&root_path),
    );

    assert!(
        plan.is_none(),
        "directory locators are search roots, not filename patterns"
    );
}

#[test]
fn scalar_path_auto_locator_does_not_use_deterministic_plan_for_directory_search_scope() {
    let root = TempDirGuard::new("scalar_auto_locator_search_scope");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write report");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert!(scalar_path_auto_locator_deterministic_plan_result(
        "find a named item inside the resolved directory",
        Some(&route),
        &loop_state,
        Some(&root_path),
    )
    .is_none());
}

#[test]
fn scalar_path_directory_locator_search_uses_structural_name_target() {
    let root = TempDirGuard::new("scalar_auto_locator_search_target");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write report");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = scalar_path_directory_locator_search_deterministic_plan_result(
        "find a named item inside the resolved directory",
        Some(&route),
        &loop_state,
        Some(&root_path),
        &format!("去 {root_path} 找 abcd，只输出路径"),
    )
    .expect("directory-scoped scalar path lookup should not need LLM planning");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("root").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(args.get("pattern").and_then(Value::as_str), Some("abcd"));
            assert_eq!(args.get("target_kind").and_then(Value::as_str), Some("any"));
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_path_directory_locator_search_resolves_unique_entry_token_without_phrase_matching() {
    let root = TempDirGuard::new("scalar_auto_locator_search_unique_token");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write target");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = scalar_path_directory_locator_search_deterministic_plan_result(
        "find a named item inside the resolved directory",
        Some(&route),
        &loop_state,
        Some(&root_path),
        &format!("Inside {root_path}, find abcd and return only the path"),
    )
    .expect("unique existing token should define the directory-scoped lookup target");

    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(args.get("pattern").and_then(Value::as_str), Some("abcd"));
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_path_auto_locator_directory_builds_observation_plan() {
    let root = TempDirGuard::new("scalar_auto_locator_dir");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.delivery_required = false;

    let actions =
        scalar_path_auto_locator_observation_plan(Some(&route), Some(&root_path)).unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([root_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn generic_directory_auto_locator_builds_inventory_synthesis_plan() {
    let root = TempDirGuard::new("generic_dir_auto_locator");
    fs::write(root.path.join("small.log"), "x").expect("write small");
    fs::write(root.path.join("large.log"), "xxxxxx").expect("write large");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = String::new();
    route.output_contract.delivery_required = false;

    let actions =
        generic_directory_auto_locator_observation_plan(Some(&route), Some(root_path.as_str()))
            .expect("directory route should build a default observation plan");

    assert_eq!(actions.len(), 3);
    match &actions[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(
                args.get("sort_by").and_then(Value::as_str),
                Some("size_desc")
            );
            assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
        }
        other => panic!("expected fs_basic list_dir action, got {other:?}"),
    }
    assert!(matches!(actions[1], AgentAction::SynthesizeAnswer { .. }));
    assert!(matches!(actions[2], AgentAction::Respond { .. }));
}

#[test]
fn directory_entry_groups_auto_locator_uses_fs_basic_list_dir() {
    let root = TempDirGuard::new("directory_entry_groups_auto_locator");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    let plan = directory_entry_groups_auto_locator_deterministic_plan_result(
        &test_state(),
        "group directory entries",
        Some(&route),
        &LoopState::new(1),
        "按文件和文件夹分组",
        Some("按文件和文件夹分组"),
        Some(root_path.as_str()),
    )
    .expect("directory entry groups plan should be available");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { tool, args })
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("list_dir")
                && args.get("path").and_then(Value::as_str) == Some(root_path.as_str())
                && args.get("names_only").and_then(Value::as_bool) == Some(false)
                && args.get("sort_by").and_then(Value::as_str) == Some("mtime_desc")
    ));
}

#[test]
fn generic_directory_auto_locator_uses_mtime_for_directory_entry_groups() {
    let root = TempDirGuard::new("generic_dir_entry_group_auto_locator");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    let actions =
        generic_directory_auto_locator_observation_plan(Some(&route), Some(root_path.as_str()))
            .expect("directory entry group fallback should build an observation plan");

    assert_eq!(actions.len(), 3);
    match &actions[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(
                args.get("sort_by").and_then(Value::as_str),
                Some("mtime_desc")
            );
            assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
        }
        other => panic!("expected fs_basic list_dir action, got {other:?}"),
    }
}

#[test]
fn directory_entry_groups_rewrites_tree_summary_to_list_dir() {
    let root = TempDirGuard::new("directory_entry_groups_rewrite");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "tree_summary",
            "path": root_path,
            "max_depth": 2
        }),
    }];

    let rewritten =
        rewrite_directory_entry_groups_tree_summary_to_list_dir(Some(&route), None, actions);

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("list_dir")
                && args.get("names_only").and_then(Value::as_bool) == Some(false)
    ));
}

#[test]
fn directory_names_contract_overrides_planner_hidden_inventory() {
    let root = TempDirGuard::new("directory_names_hidden_override");
    fs::create_dir_all(root.path.join(".cache")).expect("create hidden dir");
    fs::create_dir_all(root.path.join("docs")).expect("create docs dir");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": root_path,
            "dirs_only": true,
            "include_hidden": true,
            "names_only": true
        }),
    }];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "list directory names except ignored hidden VCS internals",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn directory_tree_auto_locator_deterministic_plan_uses_system_basic_tree_summary() {
    let root = TempDirGuard::new("directory_tree_auto_locator");
    fs::create_dir_all(root.path.join("archive")).expect("create archive dir");
    fs::write(root.path.join("archive").join("README.txt"), "archive").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    let plan = directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "summarize directory structure",
        Some(&route),
        &LoopState::new(1),
        "summarize directory structure",
        Some("summarize directory structure"),
        Some(&root_path),
    )
    .expect("directory tree plan should be available");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallSkill { skill, args })
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("tree_summary")
                && args.get("path").and_then(Value::as_str) == Some(root_path.as_str())
    ));
}

#[test]
fn directory_purpose_auto_locator_keeps_synthesis_after_tree_summary() {
    let root = TempDirGuard::new("directory_purpose_auto_locator");
    fs::create_dir_all(root.path.join("docs")).expect("create docs dir");
    fs::write(root.path.join("docs").join("README.txt"), "docs").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    let plan = directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "summarize directory purpose",
        Some(&route),
        &LoopState::new(1),
        "summarize directory purpose",
        Some("summarize directory purpose"),
        Some(&root_path),
    )
    .expect("directory purpose plan should be available");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 3);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallSkill { skill, args })
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("tree_summary")
    ));
    assert!(matches!(
        plan.steps.get(1).and_then(|step| step.to_agent_action()),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
    assert!(matches!(
        plan.steps.get(2).and_then(|step| step.to_agent_action()),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn directory_purpose_extension_locator_uses_find_entries_not_tree_summary() {
    let root = TempDirGuard::new("directory_purpose_extension_locator");
    fs::write(root.path.join("Cargo.toml"), "[workspace]\n").expect("write cargo");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "*.toml".to_string();

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "summarize representative toml files",
        Some(&route),
        &LoopState::new(1),
        "summarize representative toml files",
        Some("summarize representative toml files"),
        Some(&root_path),
    )
    .is_none());

    let plan = directory_purpose_extension_inventory_deterministic_plan_result(
        "summarize representative toml files",
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
    )
    .expect("directory purpose extension inventory plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn directory_purpose_reads_representative_found_files_after_extension_inventory() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let root = TempDirGuard::new("directory_purpose_representative_reads");
    fs::create_dir_all(root.path.join("configs/channels")).expect("create config dirs");
    let cargo_path = root.path.join("Cargo.toml");
    let config_path = root.path.join("configs/config.toml");
    let channel_path = root.path.join("configs/channels/telegram.toml");
    fs::write(&cargo_path, "[workspace]\n").expect("write cargo");
    fs::write(&config_path, "[skills]\n").expect("write config");
    fs::write(&channel_path, "[telegram]\n").expect("write channel");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "*.toml".to_string();
    let mut loop_state = LoopState::new(3);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "find_ext",
                "count": 4,
                "ext": "toml",
                "results": [
                    "Cargo.toml",
                    "configs/config.toml",
                    "configs/channels/telegram.toml",
                    "missing.toml"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let plan = directory_purpose_representative_reads_after_find_result(
        "summarize representative toml files",
        Some(&route),
        &loop_state,
        Some(&root_path),
    )
    .expect("representative read plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 5);
    let expected = [
        cargo_path.canonicalize().unwrap(),
        config_path.canonicalize().unwrap(),
        channel_path.canonicalize().unwrap(),
    ];
    for (idx, expected_path) in expected.iter().enumerate() {
        let action = plan.steps[idx].to_agent_action().expect("agent action");
        let args = expect_planned_call(&action, "fs_basic", "read_text_range");
        let expected_path = expected_path.display().to_string();
        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(expected_path.as_str())
        );
        assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
    }
    assert!(matches!(
        plan.steps.get(3).and_then(|step| step.to_agent_action()),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string()
            ]
    ));
    assert!(matches!(
        plan.steps.get(4).and_then(|step| step.to_agent_action()),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn directory_tree_auto_locator_does_not_override_exact_file_names_contract() {
    let root = TempDirGuard::new("directory_tree_auto_locator_file_names");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "list file names",
        Some(&route),
        &LoopState::new(1),
        "list file names",
        Some("list file names"),
        Some(&root_path),
    )
    .is_none());
}

#[test]
fn directory_tree_auto_locator_does_not_override_raw_command_output_contract() {
    let root = TempDirGuard::new("directory_tree_auto_locator_raw_command");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "show current process output",
        Some(&route),
        &LoopState::new(1),
        "show current process output",
        Some("show current process output"),
        Some(&root_path),
    )
    .is_none());
}

#[test]
fn directory_tree_auto_locator_does_not_override_multi_directory_contract() {
    let root = TempDirGuard::new("directory_tree_auto_locator_multi_dir");
    fs::create_dir_all(root.path.join("left")).expect("create left");
    fs::create_dir_all(root.path.join("right")).expect("create right");
    let left_path = root.path.join("left").display().to_string();
    let right_path = root.path.join("right").display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{left_path} | {right_path}");

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "compare two directories",
        Some(&route),
        &LoopState::new(1),
        "compare two directories",
        Some("compare two directories"),
        Some(&left_path),
    )
    .is_none());
}

#[test]
fn scalar_path_respond_only_uses_auto_locator_observation() {
    let root = TempDirGuard::new("scalar_auto_locator_respond_only");
    let report = root.path.join("Report.MD");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::Respond {
        content: report_path.clone(),
    }];

    let normalized = replace_scalar_path_respond_only_with_auto_locator_observation(
        Some(&route),
        &LoopState::new(1),
        Some(&report_path),
        actions,
    );
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([report_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn content_excerpt_summary_inserts_auto_locator_read_before_synthesis() {
    let root = TempDirGuard::new("content_excerpt_auto_locator");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["definitely_missing_rustclaw_20260510.md"],
                "include_missing": true
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = ensure_content_excerpt_summary_has_bounded_content(
        Some(&route),
        &loop_state,
        Some(&readme_path),
        actions,
    );

    assert_eq!(normalized.len(), 4);
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str) == Some(readme_path.as_str())
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn workspace_synthesis_respond_only_with_generic_semantic_uses_default_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::Respond {
        content: "RustClaw 是一个本地智能助手平台。".to_string(),
    }];

    let normalized =
        replace_workspace_synthesis_respond_only_plan(Some(&route), &LoopState::new(1), actions);

    assert_eq!(normalized.len(), 6);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("workspace_glance")
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_fields")
    ));
    assert!(matches!(
        &normalized[3],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
    ));
    assert!(matches!(
        &normalized[4],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
                "step_4".to_string(),
            ]
    ));
}

#[test]
fn content_excerpt_summary_auto_locator_deterministic_plan_uses_doc_parse_for_loose_doc() {
    let root = TempDirGuard::new("content_excerpt_deterministic_plan");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        "summarize a resolved fallback document",
        Some(&route),
        &loop_state,
        Some(&readme_path),
    )
    .expect("content excerpt summary should parse the resolved document directly");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "doc_parse");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("parse_doc")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(readme_path.as_str())
            );
        }
        other => panic!("expected doc_parse parse_doc action, got {other:?}"),
    }
}

#[test]
fn generic_single_document_synthesis_rewrites_bounded_read_to_doc_parse() {
    let root = TempDirGuard::new("generic_doc_parse_synthesis");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let state = test_state_with_enabled_skills(&["doc_parse", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": readme_path.clone(),
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "parse README and summarize the key points",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "doc_parse"
                && args.get("action").and_then(Value::as_str) == Some("parse_doc")
                && args.get("path").and_then(Value::as_str) == Some(readme_path.as_str())
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn generic_single_log_synthesis_rewrites_bounded_read_to_log_analyze() {
    let root = TempDirGuard::new("generic_log_analyze_synthesis");
    let log = root.path.join("app.log");
    fs::write(
        &log,
        "INFO boot ok\nWARN latency high\nERROR provider timeout\nINFO retry ok\n",
    )
    .expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": log_path.clone(),
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "analyze this log briefly",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "log_analyze"
                && args.get("path").and_then(Value::as_str) == Some(log_path.as_str())
                && args.get("max_matches").and_then(Value::as_u64) == Some(50)
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn content_excerpt_summary_keeps_bounded_log_read_for_synthesis() {
    let root = TempDirGuard::new("content_excerpt_log_read_synthesis");
    let log = root.path.join("model_io.log");
    fs::write(
        &log,
        "INFO boot ok\nWARN latency high\nERROR provider timeout\nINFO retry ok\n",
    )
    .expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": log_path.clone(),
                "mode": "tail",
                "n": 4
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "summarize the last log lines",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str) == Some(log_path.as_str())
                && args.get("mode").and_then(Value::as_str) == Some("tail")
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn content_excerpt_contract_rewrites_concrete_respond_after_synthesis() {
    let state = test_state_with_registry();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some("observed tail evidence".to_string());
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "stale concrete summary".to_string(),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "summarize observed excerpt",
        None,
        actions,
    );

    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn content_excerpt_summary_auto_locator_deterministic_plan_uses_fs_basic_for_repo_prompt_doc() {
    let root = TempDirGuard::new("content_excerpt_repo_prompt_deterministic_plan");
    let prompt_dir = root.path.join("prompts/layers/generated/skills");
    fs::create_dir_all(&prompt_dir).expect("create prompt dir");
    let prompt_file = prompt_dir.join("fs_basic.md");
    fs::write(
        &prompt_file,
        "## fs_basic\n\nFilesystem facts and bounded reads.",
    )
    .expect("write prompt file");
    let prompt_path = prompt_file.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        "summarize a generated skill prompt",
        Some(&route),
        &loop_state,
        Some(&prompt_path),
    )
    .expect("repo prompt artifact should use a bounded filesystem read");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("read_text_range")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(prompt_path.as_str())
            );
        }
        other => panic!("expected fs_basic read_text_range action, got {other:?}"),
    }
}

#[test]
fn content_excerpt_with_summary_does_not_use_head_read_deterministic_plan() {
    let root = TempDirGuard::new("content_excerpt_with_summary_no_deterministic_plan");
    let log = root.path.join("model_io.log");
    fs::write(&log, "line 1\nline 2\nline 3\nline 4\n").expect("write log");
    let log_path = log.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert!(
        content_excerpt_summary_auto_locator_deterministic_plan_result(
            "show a bounded excerpt and summarize it",
            Some(&route),
            &loop_state,
            Some(&log_path),
        )
        .is_none()
    );
}

#[test]
fn scalar_content_auto_locator_skips_content_excerpt_with_summary_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_skips_content_excerpt");
    let log = root.path.join("model_io.log");
    fs::write(&log, "line 1\nline 2\nline 3\nline 4\n").expect("write log");
    let log_path = log.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    assert!(scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "show a bounded excerpt and summarize it",
        Some(&route),
        &loop_state,
        "show the last 4 lines and summarize recovery status",
        Some("show the last 4 lines and summarize recovery status"),
        Some(&log_path),
    )
    .is_none());
}

#[test]
fn generic_content_evidence_does_not_use_single_file_deterministic_plan() {
    let root = TempDirGuard::new("generic_content_evidence_no_deterministic_plan");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert!(
        content_excerpt_summary_auto_locator_deterministic_plan_result(
            "summarize a resolved local document",
            Some(&route),
            &loop_state,
            Some(&readme_path),
        )
        .is_none()
    );
}

#[test]
fn structured_scalar_compare_does_not_use_single_file_content_deterministic_plan() {
    let root = TempDirGuard::new("structured_scalar_no_single_content_deterministic_plan");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert!(
        content_excerpt_summary_auto_locator_deterministic_plan_result(
            "compare files",
            Some(&route),
            &loop_state,
            Some(&readme_path),
        )
        .is_none()
    );
}

#[test]
fn scalar_content_auto_locator_does_not_read_path_only_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator");
    let note = root.path.join("service_notes.md");
    fs::write(&note, "# Reading Notes\n\nService status is healthy.").expect("write note");
    let note_path = note.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    assert!(scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "extract scalar from resolved file content",
        Some(&route),
        &loop_state,
        "extract scalar from resolved file content",
        Some("extract scalar from resolved file content"),
        Some(&note_path),
    )
    .is_none());
}

#[test]
fn scalar_content_auto_locator_does_not_read_existence_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_existence");
    let note = root.path.join("package.json");
    fs::write(&note, r#"{"name":"fixture"}"#).expect("write package");
    let note_path = note.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    assert!(scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "check whether the file exists",
        Some(&route),
        &loop_state,
        "check whether the file exists",
        Some("check whether the file exists"),
        Some(&note_path),
    )
    .is_none());
}

#[test]
fn scalar_content_auto_locator_reads_generic_scalar_content_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_generic");
    let note = root.path.join("service_notes.md");
    fs::write(&note, "# Reading Notes\n\nService status is healthy.").expect("write note");
    let note_path = note.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "extract scalar from resolved file content",
        Some(&route),
        &loop_state,
        "extract scalar from resolved file content",
        Some("extract scalar from resolved file content"),
        Some(&note_path),
    )
    .expect("generic content-evidence scalar contracts should read the resolved file");

    assert_eq!(plan.steps.len(), 3);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str) == Some(note_path.as_str())
    ));
    assert!(matches!(
        plan.steps[1].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn scalar_content_auto_locator_validates_config_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_config_validation");
    let config = root.path.join("config.toml");
    fs::write(&config, "[service]\nname = \"rustclaw\"\n").expect("write config");
    let config_path = config.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "validate structured config syntax",
        Some(&route),
        &loop_state,
        "validate structured config syntax",
        Some("validate structured config syntax"),
        Some(&config_path),
    )
    .expect("config validation should use structured validation");

    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("validate")
                && args.get("path").and_then(Value::as_str) == Some(config_path.as_str())
                && args.get("validation_profile").and_then(Value::as_str)
                    == Some("syntax_only")
    ));
}

#[test]
fn scalar_content_auto_locator_uses_structured_read_field_for_structured_scalar_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_structured_field");
    let manifest = root.path.join("Cargo.toml");
    fs::write(&manifest, "[package]\nname = \"rustclaw-test\"\n").expect("write manifest");
    let manifest_path = manifest.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = manifest_path.clone();
    route.resolved_intent =
        "Read package.name from Cargo.toml and output only that value.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "Read package.name from Cargo.toml and output only that value.",
        Some(&route),
        &loop_state,
        "Read package.name from Cargo.toml and output only that value.",
        Some("Read package.name from Cargo.toml and output only that value."),
        Some(&manifest_path),
    )
    .expect("structured scalar contracts should use structured field reads");

    assert_eq!(plan.steps.len(), 1);
    let actual = plan.steps[0].to_agent_action();
    assert!(
        matches!(
        actual,
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str) == Some(manifest_path.as_str())
                && args.get("field_path").and_then(Value::as_str) == Some("package.name")
        ),
        "unexpected plan action: {:?}",
        actual
    );
}

#[test]
fn scalar_content_auto_locator_resolves_cargo_workspace_inherited_package_version() {
    let root = TempDirGuard::new("scalar_content_auto_locator_workspace_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace manifest");
    let member_dir = root.path.join("crates/clawd");
    fs::create_dir_all(&member_dir).expect("create member");
    fs::write(
        member_dir.join("Cargo.toml"),
        r#"[package]
name = "clawd"
version.workspace = true
"#,
    )
    .expect("write member manifest");
    let member_manifest = member_dir.join("Cargo.toml");
    let member_path = member_manifest.display().to_string();
    let root_manifest = root.path.join("Cargo.toml").display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = member_path.clone();
    route.resolved_intent =
        "Read package.version from crates/clawd/Cargo.toml and output only the value.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "Read package.version from crates/clawd/Cargo.toml and output only the value.",
        Some(&route),
        &loop_state,
        "Read package.version from crates/clawd/Cargo.toml and output only the value.",
        Some("Read package.version from crates/clawd/Cargo.toml and output only the value."),
        Some(&member_path),
    )
    .expect("workspace-inherited Cargo scalar contracts should read workspace package field");

    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str) == Some(root_manifest.as_str())
                && args.get("field_path").and_then(Value::as_str)
                    == Some("workspace.package.version")
    ));
}

#[test]
fn scalar_content_auto_locator_ignores_memory_field_when_current_request_names_bare_key() {
    let root = TempDirGuard::new("scalar_content_auto_locator_bare_key");
    let fixture_dir = root.path.join("scripts/nl_tests/fixtures/device_local");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let package = fixture_dir.join("package.json");
    fs::write(
        &package,
        r#"{
  "name": "rustclaw-nl-fixture",
  "version": "1.0.0",
  "scripts": { "build": "echo build" }
}"#,
    )
    .expect("write package");
    let package_path = package.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = package_path.clone();
    route.resolved_intent =
            "Extract the name field from scripts/nl_tests/fixtures/device_local/package.json and output only the value."
                .to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let current_request =
        "读取 scripts/nl_tests/fixtures/device_local/package.json 的 name 字段，只输出值。";
    let goal = format!(
            "### PLANNER_MEMORY_CONTEXT\nfixture fact: scripts.build='echo build'\n\n### CURRENT_REQUEST\n{current_request}"
        );

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        &goal,
        Some(&route),
        &loop_state,
        current_request,
        Some(current_request),
        Some(&package_path),
    )
    .expect("bare schema key should be selected from current request");

    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str) == Some(package_path.as_str())
                && args.get("field_path").and_then(Value::as_str) == Some("name")
    ));
}

#[test]
fn scalar_path_respond_only_uses_loop_state_auto_locator_observation() {
    let root = TempDirGuard::new("scalar_auto_locator_loop_state");
    let report = root.path.join("Report.MD");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::Respond {
        content: report_path.clone(),
    }];
    let mut loop_state = LoopState::new(1);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), report_path.clone());

    let normalized = replace_scalar_path_respond_only_with_auto_locator_observation(
        Some(&route),
        &loop_state,
        None,
        actions,
    );
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([report_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn scalar_count_synthesis_only_uses_count_inventory_for_locator_dir() {
    let root = TempDirGuard::new("scalar_count_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::write(root.path.join("b.txt"), "b").expect("write b");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count entries",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_listing_plan_uses_count_inventory_for_locator_dir() {
    let root = TempDirGuard::new("scalar_count_listing_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::write(root.path.join("b.txt"), "b").expect("write b");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": root_path.clone()}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count entries",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_listing_plan_preserves_dirs_only_dimension_for_count_inventory() {
    let root = TempDirGuard::new("scalar_count_dirs_only_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "count_entries",
                "path": root_path.clone(),
                "dirs_only": true,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count directories",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(args.get("kind_filter").and_then(Value::as_str), Some("dir"));
            assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(true));
            assert_eq!(
                args.get("count_files").and_then(Value::as_bool),
                Some(false)
            );
            assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_listing_plan_preserves_files_kind_for_count_inventory() {
    let root = TempDirGuard::new("scalar_count_files_only_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": root_path.clone(),
            "kind": "files",
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count files",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("kind_filter").and_then(Value::as_str),
                Some("file")
            );
            assert_eq!(args.get("count_files").and_then(Value::as_bool), Some(true));
            assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(false));
            assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_listing_plan_preserves_extension_filter_for_count_inventory() {
    let root = TempDirGuard::new("scalar_count_ext_filter_locator_dir");
    fs::write(root.path.join("a.md"), "a").expect("write a");
    fs::write(root.path.join("b.txt"), "b").expect("write b");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": root_path.clone(),
            "ext_filter": "md",
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count markdown files",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("kind_filter").and_then(Value::as_str),
                Some("file")
            );
            assert_eq!(args.get("ext_filter").and_then(Value::as_str), Some("md"));
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_repair_preserves_explicit_count_path_over_auto_locator() {
    let root = TempDirGuard::new("scalar_count_explicit_over_auto_locator");
    fs::create_dir_all(root.path.join(".git")).expect("create .git");
    fs::create_dir_all(root.path.join("crates")).expect("create crates");
    let root_path = root.path.display().to_string();
    let git_path = root.path.join(".git").display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "count_entries",
                "path": root_path.clone(),
                "include_hidden": false,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&git_path),
        "count top-level entries except the hidden git directory",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_ne!(
                args.get("path").and_then(Value::as_str),
                Some(git_path.as_str())
            );
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(false)
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_unqualified_listing_plan_forces_structured_count_repair() {
    let root = TempDirGuard::new("scalar_count_unqualified_listing");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": root_path.clone(),
                "names_only": true,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count entries",
        actions,
    );

    assert_eq!(normalized.len(), 3);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
        }
        other => panic!("expected preserved fs_basic list_dir action, got {other:?}"),
    }
    let state = test_state();
    let loop_state = LoopState::new(1);
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "scalar_count_requires_structured_count_action"
    );
}

#[test]
fn scalar_count_missing_explicit_path_checks_that_path_not_auto_parent() {
    let root = TempDirGuard::new("scalar_count_missing_explicit_path");
    let parent = root.path.join("configs");
    fs::create_dir_all(&parent).expect("create parent");
    let parent_path = parent.display().to_string();
    let missing = root.path.join("configs/config_copy");
    let missing_path = missing.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = missing_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": missing_path.clone(),
            "ext_filter": "toml"
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&parent_path),
        "查一下目录下有几个 toml 文件",
        actions,
    );

    assert_eq!(normalized.len(), 2);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([missing_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
    match &normalized[1] {
        AgentAction::Respond { content } => {
            assert!(content.contains("不存在"));
            assert!(content.contains("无法统计"));
        }
        other => panic!("expected missing-path Respond action, got {other:?}"),
    }
}

#[test]
fn observed_missing_read_file_reply_does_not_force_plan_repair() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let missing_path =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/missing.md";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "read_file".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!("__RC_READ_FILE_NOT_FOUND__:{missing_path}")),
        started_at: 1,
        finished_at: 2,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = format!("读取 {missing_path}；如果不存在，只回答“不存在”和这个路径");
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = missing_path.to_string();
    let actions = vec![AgentAction::Respond {
        content: format!("不存在\n{missing_path}"),
    }];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn scalar_count_pathlike_hint_in_current_workspace_does_not_use_parent_auto_locator() {
    let root = TempDirGuard::new("scalar_count_pathlike_current_workspace");
    let parent = root.path.join("configs");
    fs::create_dir_all(&parent).expect("create parent");
    let parent_path = parent.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "configs/config_copy".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "configs/config_copy",
            "ext_filter": "toml"
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&parent_path),
        "查一下目录下有几个 toml 文件",
        actions,
    );

    assert_eq!(normalized.len(), 2);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!(["configs/config_copy"])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
    match &normalized[1] {
        AgentAction::Respond { content } => {
            assert!(content.contains("不存在"));
            assert!(content.contains("无法统计"));
        }
        other => panic!("expected missing-path Respond action, got {other:?}"),
    }
}

#[test]
fn hidden_entries_scalar_contract_uses_inventory_dir() {
    let root = TempDirGuard::new("hidden_entries_scalar_plan");
    fs::write(root.path.join(".env"), "a").expect("write hidden");
    fs::write(root.path.join("visible.txt"), "b").expect("write visible");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path": root_path.clone()}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "list_dir"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "current workspace hidden entries check",
        None,
        Some(&root_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_dir")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn hidden_entries_scalar_current_workspace_hint_falls_back_to_dot_inventory() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "current directory".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "find . -maxdepth 1 -name '.*' | wc -l"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "count hidden entries in current directory",
        None,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_dir")
            );
            assert_eq!(args.get("path").and_then(|value| value.as_str()), Some("."));
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(true)
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn service_status_contract_rewrites_pgrep_run_cmd_to_service_control_status() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pgrep -x telegramd > /dev/null && echo 'running' || echo 'not running'"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "service_control");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("status"));
            assert_eq!(
                args.get("target").and_then(Value::as_str),
                Some("telegramd")
            );
            assert!(args.get("manager_type").is_none());
        }
        other => panic!("expected service_control status action, got {other:?}"),
    }
    assert_eq!(normalized.len(), 1);
}

#[test]
fn service_status_contract_rewrites_pgrep_script_without_trailing_shell_words() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "pgrep -fa telegramd 2>/dev/null; if [ $? -ne 0 ]; then echo 'telegramd is NOT currently running'; else echo 'telegramd is currently running'; fi"}),
    }];

    let normalized = rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "service_control");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("status"));
            assert_eq!(
                args.get("target").and_then(Value::as_str),
                Some("telegramd")
            );
        }
        other => panic!("expected service_control status action, got {other:?}"),
    }
}

#[test]
fn service_status_contract_rewrites_systemctl_status_to_service_control_systemd() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "run_cmd",
            "command": "systemctl is-active nginx.service"
        }),
    }];

    let normalized = rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "service_control");
            assert_eq!(
                args.get("target").and_then(Value::as_str),
                Some("nginx.service")
            );
            assert_eq!(
                args.get("manager_type").and_then(Value::as_str),
                Some("systemd")
            );
        }
        other => panic!("expected service_control status action, got {other:?}"),
    }
}

#[test]
fn normalize_prefers_registry_repair_over_legacy_service_rewrite() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl status clawd"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "check clawd service status",
        None,
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "preferred_skill_required_for_semantic_route"
    );
}

#[test]
fn normalize_prefers_registry_sqlite_rewrite_over_text_read_fallback() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({"path": "data/db-basic-contract.sqlite"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "list sqlite tables",
        None,
        Some("data/db-basic-contract.sqlite"),
        actions,
    );

    assert!(planned_call_is(&normalized[0], "db_basic", "list_tables"));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn normalize_prefers_registry_repair_over_legacy_docker_rewrite() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DockerPs;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker ps"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "show docker containers",
        None,
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "preferred_skill_required_for_semantic_route"
    );
}

#[test]
fn normalize_prefers_registry_repair_over_legacy_archive_unpack_rewrite() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.locator_hint = "/tmp/source.tgz | /tmp/source-unpacked".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "tar -xzf /tmp/source.tgz -C /tmp/source-unpacked"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "unpack archive",
        None,
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "preferred_skill_required_for_semantic_route"
    );
}

#[test]
fn explicit_service_command_is_preserved_as_run_cmd() {
    let mut state = test_state_with_enabled_skills(&["service_control", "run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行命令 ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "service_control".to_string(),
        args: json!({
            "action": "status",
            "target": "clawd"
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "执行命令 systemctl status clawd --no-pager，告诉我结果",
        Some("执行命令 systemctl status clawd --no-pager，告诉我结果"),
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("systemctl status clawd --no-pager")
            );
        }
        other => panic!("expected preserved run_cmd action, got {other:?}"),
    }
}

#[test]
fn observed_judgment_mixed_placeholder_respond_uses_synthesize_after_listing() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path": "document", "limit": 5}),
        },
        AgentAction::Respond {
            content:
                "Here are the first files:\n{{last_output}}\nThese look more like documentation."
                    .to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["list_dir"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list files and judge their role",
        None,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(planned_call_is(&normalized[0], "fs_basic", "list_dir"));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn scalar_count_preserves_planned_run_cmd_observation() {
    let root = TempDirGuard::new("scalar_count_run_cmd_plan");
    fs::write(root.path.join(".env"), "a").expect("write hidden");
    fs::write(root.path.join("visible.txt"), "b").expect("write visible");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "printf '2\\n'"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "count current workspace entries",
        None,
        Some(&root_path),
        actions,
    );

    match normalized
        .iter()
        .find(|action| matches!(action, AgentAction::CallSkill { skill, .. } if skill == "run_cmd"))
    {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(|value| value.as_str()),
                Some("printf '2\\n'")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
    assert!(!normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("count_inventory")
        )
    }));
}

#[test]
fn structured_keys_contract_rewrites_read_range_to_structured_keys() {
    let root = TempDirGuard::new("structured_keys_plan");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": config_path.clone()}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list structured keys",
        None,
        Some(&config_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "config_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("list_keys")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(config_path.as_str())
            );
        }
        other => panic!("expected config_basic list_keys action, got {other:?}"),
    }
}

#[test]
fn structured_keys_contract_rewrites_validate_to_structured_keys() {
    let root = TempDirGuard::new("structured_keys_validate_plan");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": config_path.clone(),
            "format": "toml",
            "validation_profile": "syntax_only",
        }),
    }];

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list structured keys",
        None,
        Some(&config_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

#[test]
fn structured_keys_contract_uses_deterministic_list_keys_plan() {
    let root = TempDirGuard::new("structured_keys_deterministic_plan");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let loop_state = LoopState::new(2);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list structured keys",
        "list structured keys",
        Some(&route),
        &loop_state,
        Some(&config_path),
    )
    .expect("structured keys deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.plan_kind, PlanKind::Single);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

#[test]
fn structured_keys_plan_ignores_background_field_selectors() {
    let root = TempDirGuard::new("structured_keys_background_field_plan");
    let config_path = root.path.join("config.toml");
    fs::write(
        &config_path,
        "alpha = 1\n[llm]\nselected_vendor = \"minimax\"\n",
    )
    .expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    route.resolved_intent = "list top-level keys".to_string();

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "BACKGROUND: configs/config.toml llm.selected_vendor is minimax",
        "list top-level keys",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys deterministic plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
    assert!(args.get("field_path").is_none());
}

#[test]
fn structured_keys_deterministic_plan_preserves_nested_field_path() {
    let root = TempDirGuard::new("structured_keys_nested_field_plan");
    let config_path = root.path.join("package.json");
    fs::write(
        &config_path,
        r#"{"name":"fixture","scripts":{"build":"vite","test":"vitest"}}"#,
    )
    .expect("write package json");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    route.resolved_intent = "list keys under scripts".to_string();

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list keys under scripts",
        "list keys under scripts",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys nested field plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("scripts")
    );
}

#[test]
fn structured_keys_deterministic_plan_reads_identity_scalar_field_value() {
    let root = TempDirGuard::new("structured_keys_identity_scalar_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("skills_registry.toml");
    fs::write(
        &config_path,
        r#"[[skills]]
name = "run_cmd"
enabled = true
planner_kind = "tool"

[[skills]]
name = "read_file"
enabled = true
planner_kind = "tool"
"#,
    )
    .expect("write skills registry");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    route.resolved_intent =
        "Find the run_cmd related configuration and report planner_kind.".to_string();

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "Find run_cmd planner_kind in configs/skills_registry.toml.",
        "Find run_cmd planner_kind in configs/skills_registry.toml.",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys scalar field plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("run_cmd.planner_kind")
    );
}

#[test]
fn structured_keys_retry_after_validation_uses_list_keys_plan() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let root = TempDirGuard::new("structured_keys_retry_plan");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::new(3);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "validate_structured",
                "path": config_path,
                "valid": true,
                "root_type": "object"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list structured keys",
        "list structured keys",
        Some(&route),
        &loop_state,
        Some(&config_path),
    )
    .expect("retry should collect structured keys evidence");

    assert_eq!(plan.plan_kind, PlanKind::Incremental);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

#[test]
fn structured_keys_contract_rewrites_multi_field_value_read_to_list_keys() {
    let root = TempDirGuard::new("structured_keys_multi_field_plan");
    let config_path = root.path.join("app_config.toml");
    fs::write(
        &config_path,
        "[app]\nname = \"fixture\"\n[features]\nenabled = true\n[paths]\nlogs_dir = \"logs\"\n",
    )
    .expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_fields",
            "path": config_path.clone(),
            "field_paths": ["app", "features", "paths"],
        }),
    }];

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list structured keys",
        None,
        Some(&config_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "config_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("list_keys")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(config_path.as_str())
            );
        }
        other => panic!("expected config_basic list_keys action, got {other:?}"),
    }
}

#[test]
fn structured_keys_contract_keeps_explicit_structured_field_read() {
    let root = TempDirGuard::new("structured_keys_field_read_plan");
    let config_path = root.path.join("Cargo.toml");
    fs::write(&config_path, "[package]\nname = \"clawd\"\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": config_path.clone(),
                "field_path": "package.no_such_key_100_matrix",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "config_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "read the requested structured field",
        None,
        Some(&config_path),
        actions,
    );

    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallTool { tool, args }
                if tool == "config_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_field")
                    && args.get("path").and_then(Value::as_str) == Some(config_path.as_str())
                    && args.get("field_path").and_then(Value::as_str)
                        == Some("package.no_such_key_100_matrix")
        )
    }));
    assert!(!normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallTool { tool, args }
                if tool == "config_basic"
                    && args.get("action").and_then(Value::as_str) == Some("list_keys")
        )
    }));
}

#[test]
fn file_names_route_accepts_structured_key_listing_for_structured_document() {
    let root = TempDirGuard::new("file_names_structured_keys_plan");
    let package_path = root.path.join("package.json");
    fs::write(
        &package_path,
        r#"{"scripts":{"build":"vite build","dev":"vite","lint":"eslint ."}}"#,
    )
    .expect("write package");
    let package_path = package_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = package_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": package_path,
            "field_path": "scripts",
            "max_keys": 100,
        }),
    }];

    let state = test_state_with_registry();
    assert!(!actions_use_ad_hoc_command_without_route_preferred_skill(
        &state, &route, &actions
    ));
    assert!(observation_only_plan_can_finalize_from_direct_output(
        &state,
        Some(&route),
        &actions
    ));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn plain_act_read_range_plan_uses_direct_observed_finalizer_without_synthesis() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/service_notes.md".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/tmp/service_notes.md",
            "mode": "head",
            "n": 10,
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "read first lines of /tmp/service_notes.md",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
    ));
}

#[test]
fn chat_wrapped_read_range_plan_adds_synthesis_terminal_answer() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/release_checklist.md".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/tmp/release_checklist.md",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "read /tmp/release_checklist.md and answer from its content",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn registry_prefers_config_basic_for_structured_keys_contract() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "package.json".to_string();
    let preferred = registry_preferred_skill_names_for_route(&test_state_with_registry(), &route);
    assert!(preferred.iter().any(|skill| skill == "config_basic"));
}

#[test]
fn explicit_configured_command_request_rewrites_semantic_substitute_to_run_cmd() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let loop_state = LoopState::new(1);
    let original_request = "execute ls scripts, then summarize the directory";
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/scripts",
                "names_only": true,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "list scripts and summarize the directory",
        Some(original_request),
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("request_text").and_then(Value::as_str),
                Some(original_request)
            );
            assert!(args
                .get("cwd")
                .and_then(Value::as_str)
                .is_some_and(|cwd| !cwd.trim().is_empty()));
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("ls scripts")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn explicit_command_rewrite_preserves_bounded_configured_execute_prefix() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "/workspace/scripts",
            "names_only": true,
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "explain a command",
        Some("execute ls scripts, then explain what it lists"),
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str) == Some("ls scripts")
    ));
}

#[test]
fn explicit_command_extracts_configured_standalone_command_before_freeform_tail() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];

    assert_eq!(
        super::explicit_command_segment(
            &state.policy.command_intent,
            "Run pwd and output only the raw result."
        )
        .as_deref(),
        Some("pwd")
    );
    assert_eq!(
        super::explicit_command_segment(
            &state.policy.command_intent,
            "Run cargo test and output only the raw result."
        )
        .as_deref(),
        None
    );
}

#[test]
fn explicit_command_rewrite_preserves_configured_standalone_command_before_freeform_tail() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "/workspace",
            "names_only": true,
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Get current working directory path",
        Some("Run pwd and output only the raw result."),
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str) == Some("pwd")
    ));
}

#[test]
fn multi_structured_scalar_observations_append_terminal_synthesis() {
    let state = test_state_with_enabled_skills(&["config_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "/workspace/package.json",
                "field_path": "name",
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "/workspace/crates/clawd/Cargo.toml",
                "field_path": "package.name",
            }),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "read two package names and say whether they match",
        None,
        None,
        actions,
    );

    assert!(matches!(
        normalized.get(normalized.len().saturating_sub(2)),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn scalar_path_route_treats_fs_search_query_as_name_pattern_when_action_missing() {
    let root = TempDirGuard::new("fs_search_name_contract");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "path": root_path,
            "query": "abcd",
        }),
    }];

    let normalized = enforce_output_contract_tool_args(Some(&route), actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "fs_search");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("find_name")
            );
            assert_eq!(
                args.get("pattern").and_then(|value| value.as_str()),
                Some("abcd")
            );
            assert_eq!(
                args.get("root").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_search action, got {other:?}"),
    }
}

#[test]
fn file_paths_route_preserves_grep_text_query_as_content_query() {
    let root = TempDirGuard::new("fs_search_grep_contract");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "grep_text",
            "root": root_path,
            "query": "FirstLayerDecision",
            "max_results": 3
        }),
    }];

    let normalized = enforce_output_contract_tool_args(Some(&route), actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "fs_search");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("grep_text")
            );
            assert_eq!(
                args.get("query").and_then(Value::as_str),
                Some("FirstLayerDecision")
            );
            assert!(args.get("pattern").is_none());
            assert!(args.get("ext").is_none());
        }
        other => panic!("expected fs_search action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_alias_is_normalized_to_read_range() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read",
            "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("read_range")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
            );
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_find_name_alias_is_normalized_to_find_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "find_name",
            "pattern": "missing.md",
            "max_results": 5,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("find_path")
            );
            assert_eq!(
                args.get("name").and_then(|value| value.as_str()),
                Some("missing.md")
            );
        }
        other => panic!("expected system_basic find_path action, got {other:?}"),
    }
}

#[test]
fn system_basic_check_exists_alias_is_normalized_to_path_batch_facts() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "check_exists",
            "path": "plan/extra_missing_repair_probe.md",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(
                args.get("paths").and_then(|value| value.as_array()),
                Some(&vec![json!("plan/extra_missing_repair_probe.md")])
            );
            assert!(args.get("path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn system_basic_check_exists_target_alias_keeps_batch_shape() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "check_exists",
            "target_path": "README.md",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(
                args.get("paths").and_then(|value| value.as_array()),
                Some(&vec![json!("README.md")])
            );
            assert!(args.get("target_path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn missing_read_range_path_uses_route_locator_hint() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "definitely_missing_system_basic_case.txt".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "find_path",
                "name": "definitely_missing_system_basic_case.txt",
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "mode": "head",
                "n": 3,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = fill_missing_read_range_path_from_route_locator(Some(&route), actions);
    match &normalized[1] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("definitely_missing_system_basic_case.txt")
            );
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_lines_alias_becomes_range_bounds() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "README.md",
            "lines": "1-3",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("read_range")
            );
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("range")
            );
            assert_eq!(
                args.get("start_line").and_then(|value| value.as_u64()),
                Some(1)
            );
            assert_eq!(
                args.get("end_line").and_then(|value| value.as_u64()),
                Some(3)
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(3));
            assert!(args.get("lines").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_range_tail_alias_becomes_mode_tail() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "logs/model_io.log",
            "range": "tail",
            "n": 4,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("tail")
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(4));
            assert!(args.get("range").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_line_start_alias_becomes_range_bounds() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "README.md",
            "line_start": 1,
            "line_end": 8,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("read_range")
            );
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("range")
            );
            assert_eq!(
                args.get("start_line").and_then(|value| value.as_u64()),
                Some(1)
            );
            assert_eq!(
                args.get("end_line").and_then(|value| value.as_u64()),
                Some(8)
            );
            assert!(args.get("line_start").is_none());
            assert!(args.get("line_end").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_alias_with_lines_becomes_range_bounds() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read",
            "path": "README.md",
            "lines": [2, 4],
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("read_range")
            );
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("range")
            );
            assert_eq!(
                args.get("start_line").and_then(|value| value.as_u64()),
                Some(2)
            );
            assert_eq!(
                args.get("end_line").and_then(|value| value.as_u64()),
                Some(4)
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(3));
            assert!(args.get("lines").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_negative_bounds_becomes_tail_count() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "logs/app.log",
            "start_line": -12,
            "end_line": -1,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("tail")
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(12));
            assert!(args.get("start_line").is_none());
            assert!(args.get("end_line").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_negative_start_line_count_becomes_tail_count() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "logs/model_io.log",
            "start_line": -4,
            "line_count": 4,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("tail")
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(4));
            assert!(args.get("start_line").is_none());
            assert!(args.get("line_count").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_line_count_template_becomes_tail_count() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "file_lines_count",
                "path": "logs/model_io.log",
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "logs/model_io.log",
                "start_line": "{{s1.result.line_count - 4}}",
                "end_line": "{{s1.result.line_count}}",
            }),
        },
    ];

    let normalized = strip_file_lines_count_before_tail_read_range(
        normalize_system_basic_schema_aliases(actions),
    );
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("tail")
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(5));
            assert!(args.get("start_line").is_none());
            assert!(args.get("end_line").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_list_dir_alias_is_normalized_to_inventory_dir() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "scripts/nl_tests/fixtures/device_local/docs",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("names_only").and_then(|value| value.as_bool()),
                Some(true)
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn system_basic_stat_paths_alias_is_normalized_to_path_batch_facts() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "path": "configs/channels",
            "fields": ["path", "kind"],
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(
                args.get("paths")
                    .and_then(Value::as_array)
                    .and_then(|paths| paths.first())
                    .and_then(Value::as_str),
                Some("configs/channels")
            );
            assert!(args.get("path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn system_basic_inventory_dir_dir_path_alias_becomes_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "dir_path": "scripts/nl_tests/fixtures/device_local/docs",
            "sort_by": "name",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("scripts/nl_tests/fixtures/device_local/docs")
            );
            assert!(args.get("dir_path").is_none());
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn system_basic_count_dir_alias_is_normalized_to_count_inventory() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "count_dir",
            "directory_path": "document",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("count_inventory")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("document")
            );
            assert!(args.get("directory_path").is_none());
        }
        other => panic!("expected system_basic count_inventory action, got {other:?}"),
    }
}

#[test]
fn system_basic_inventory_dir_extension_filter_implies_files_only() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "document",
            "ext_filter": ".md",
            "names_only": true,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("files_only").and_then(|value| value.as_bool()),
                Some(true)
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn system_basic_inventory_dir_normalizes_size_sort_aliases() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "logs",
            "files_only": true,
            "sort_by": "size",
            "sort_order": "desc",
            "max_entries": 3,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("sort_by").and_then(|value| value.as_str()),
                Some("size_desc")
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn hidden_entries_contract_forces_inventory_dir_include_hidden() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": ".",
            "names_only": true,
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn structured_scalar_compare_plan_appends_synthesize_answer() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_fields",
                "path": "UI/package.json",
                "field_paths": ["name"]
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": "crates/clawd/Cargo.toml",
                "field_path": "package.name"
            }),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.resolved_intent =
            "UI/package.json 里的 name 和 crates/clawd/Cargo.toml 里的 package.name 一样吗？只回答一样或不一样"
                .to_string();

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn structured_scalar_compare_repairs_whole_file_read_plan() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "UI/package.json" }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/Cargo.toml" }),
        },
    ];
    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "读取两个字段并比较",
        None,
        actions,
    );

    assert!(should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
    assert_eq!(
        plan_repair_reason(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            Some(&normalized)
        ),
        "structured_scalar_compare_requires_extract_fields"
    );
}

#[test]
fn structured_scalar_compare_repair_can_add_text_after_prior_scalar_extract() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "extract_field",
                "path": "Cargo.toml",
                "field_path": "workspace.package.version",
                "value": "0.1.7",
                "value_text": "0.1.7"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: serde_json::json!({
            "action": "grep_text",
            "root": "README.md",
            "query": "0.1.7",
            "max_results": 5
        }),
    }];

    assert!(
        !should_force_actionable_plan_repair(&test_state(), Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        plan_repair_reason(&test_state(), Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn structured_scalar_compare_keeps_two_structured_extracts_for_strict_shape() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_fields",
                "path": "UI/package.json",
                "field_paths": ["name"]
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_fields",
                "path": "crates/clawd/Cargo.toml",
                "field_paths": ["package.name"]
            }),
        },
    ];
    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "读取两个字段并比较",
        None,
        actions,
    );

    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs
                == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_accepts_two_directory_inventory_observations() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "list_dir",
                "path": "scripts/nl_tests/fixtures/device_local/docs"
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "list_dir",
                "path": "scripts/nl_tests/fixtures/device_local/logs"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string(), "s1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "先数 docs 直接子项数量，再数 logs 直接子项数量，最后一句中文说哪个更多",
        None,
        actions,
    );

    assert!(planned_call_is(&normalized[0], "fs_basic", "list_dir"));
    assert!(planned_call_is(&normalized[1], "fs_basic", "list_dir"));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_accepts_path_batch_facts_for_file_metadata() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "path_batch_facts",
            "paths": ["Cargo.lock", "Cargo.toml"]
        }),
    }];
    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "比较 Cargo.lock 和 Cargo.toml 的大小",
        None,
        actions,
    );

    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_one_sentence_accepts_path_batch_facts_metadata_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["README.md", "AGENTS.md"],
                "fields": ["size_bytes"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &actions
    ));
    assert_ne!(
        plan_repair_reason(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            Some(&actions)
        ),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn structured_scalar_compare_free_shape_accepts_path_batch_facts_metadata_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml | Cargo.lock".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["Cargo.toml", "Cargo.lock"],
                "fields": ["size_bytes"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
    assert_ne!(
        plan_repair_reason(
            &test_state(),
            Some(&route),
            &LoopState::new(1),
            Some(&actions)
        ),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn quantity_compare_rewrites_directory_name_searches_to_dir_compare() {
    let root = TempDirGuard::new("quantity_dir_compare");
    fs::create_dir_all(root.path.join("tmp/bundle_src/nested")).expect("create left");
    fs::create_dir_all(root.path.join("tmp/dynamic_guard_unpack_case/nested"))
        .expect("create right");
    fs::write(root.path.join("tmp/bundle_src/notes.txt"), "same\n").expect("write left");
    fs::write(
        root.path.join("tmp/dynamic_guard_unpack_case/notes.txt"),
        "same\n",
    )
    .expect("write right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.locator_scan_max_files = 5000;

    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root.path.display().to_string();

    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": ".",
                "pattern": "bundle_src"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": ".",
                "pattern": "dynamic_guard_unpack_case"
            }),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(2),
        "compare bundle_src and dynamic_guard_unpack_case recursively",
        None,
        None,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "system_basic", "dir_compare");
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some("tmp/bundle_src")
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some("tmp/dynamic_guard_unpack_case")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn quantity_compare_directory_pair_uses_deterministic_dir_compare_plan() {
    let root = TempDirGuard::new("quantity_dir_compare_locator");
    let left = root.path.join("tmp/bundle_src");
    let right = root.path.join("tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{} | {}", left.display(), right.display());

    let plan = directory_compare_locator_deterministic_plan_result(
        &state,
        "compare two directories recursively",
        Some(&route),
        &LoopState::new(1),
    )
    .expect("deterministic dir compare plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("action");
    let args = expect_planned_call(&action, "system_basic", "dir_compare");
    let expected_left = left.canonicalize().unwrap().display().to_string();
    let expected_right = right.canonicalize().unwrap().display().to_string();
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(expected_left.as_str())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(expected_right.as_str())
    );
}

#[test]
fn directory_pair_locator_uses_dir_compare_even_without_quantity_semantic() {
    let root = TempDirGuard::new("directory_pair_compare_locator_no_semantic");
    let left = root.path.join("tmp/bundle_src");
    let right = root.path.join("tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{} | {}", left.display(), right.display());

    let plan = directory_compare_locator_deterministic_plan_result(
        &state,
        "compare two directory targets",
        Some(&route),
        &LoopState::new(1),
    )
    .expect("deterministic dir compare plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("action");
    let args = expect_planned_call(&action, "system_basic", "dir_compare");
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(left.canonicalize().unwrap().to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(right.canonicalize().unwrap().to_string_lossy().as_ref())
    );
}

#[test]
fn dir_compare_plan_rewrites_unique_directory_basenames_to_paths() {
    let root = TempDirGuard::new("dir_compare_unique_basename_rewrite");
    let left = root.path.join("fixtures/tmp/bundle_src");
    let right = root.path.join("fixtures/tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "dir_compare",
            "left_path": "bundle_src",
            "right_path": "dynamic_guard_unpack_case",
            "recursive": true,
            "max_diffs": 20,
        }),
    }];

    let normalized =
        super::normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "system_basic", "dir_compare");
    let expected_left = left.canonicalize().unwrap().display().to_string();
    let expected_right = right.canonicalize().unwrap().display().to_string();
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(expected_left.as_str())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(expected_right.as_str())
    );
}

#[test]
fn compare_paths_plan_rewrites_to_system_dir_compare_with_resolved_dirs() {
    let root = TempDirGuard::new("compare_paths_to_dir_compare_rewrite");
    for idx in 0..2500 {
        fs::create_dir_all(root.path.join(format!("aaa_filler_{idx:04}"))).expect("filler");
    }
    let left = root.path.join("fixtures/tmp/bundle_src");
    let right = root.path.join("fixtures/tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.locator_scan_max_files = 10;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "paths": ["bundle_src", "dynamic_guard_unpack_case"],
            "recursive": true
        }),
    }];

    let normalized =
        super::normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "system_basic", "dir_compare");
    let expected_left = left.canonicalize().unwrap().display().to_string();
    let expected_right = right.canonicalize().unwrap().display().to_string();
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(expected_left.as_str())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(expected_right.as_str())
    );
    assert!(args.get("paths").is_none());
}

#[test]
fn constructed_missing_stat_path_plan_rewrites_to_exact_find_entries() {
    let root = TempDirGuard::new("constructed_missing_stat_path");
    let locator = root.path.join("locator_smart");
    fs::create_dir_all(locator.join("case_only")).expect("create case dir");
    fs::write(locator.join("case_only/Report.MD"), "").expect("write report");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/Report.MD"]
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "在 locator_smart 目录下查找 Report.MD 文件，仅输出路径",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("locator_smart")
    );
    assert_eq!(
        args.get("name_pattern").and_then(Value::as_str),
        Some("Report.MD")
    );
    assert_eq!(args.get("exact").and_then(Value::as_bool), Some(true));
}

#[test]
fn constructed_missing_stat_path_plan_rewrites_without_specific_semantic_kind() {
    let root = TempDirGuard::new("constructed_missing_stat_path_generic");
    let locator = root.path.join("locator_smart");
    fs::create_dir_all(locator.join("case_only")).expect("create case dir");
    fs::write(locator.join("case_only/Report.MD"), "").expect("write report");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/Report.MD"]
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "在目录 locator_smart 中查找文件 Report.MD 并输出其完整路径",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("name_pattern").and_then(Value::as_str),
        Some("Report.MD")
    );
}

#[test]
fn constructed_directory_stat_path_plan_rewrites_to_find_entries_for_child_selector() {
    let root = TempDirGuard::new("constructed_directory_stat_path");
    let locator = root.path.join("locator_smart/fuzzy_top3");
    fs::create_dir_all(&locator).expect("create locator dir");
    fs::write(locator.join("abcd_report.md"), "").expect("write report");
    fs::write(locator.join("my_abcd.txt"), "").expect("write text");
    fs::write(locator.join("zz_abcd_backup.log"), "").expect("write log");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/fuzzy_top3"]
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "去 locator_smart/fuzzy_top3 找 abcd",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("locator_smart/fuzzy_top3")
    );
    assert_eq!(
        args.get("name_pattern").and_then(Value::as_str),
        Some("abcd")
    );
    assert_eq!(args.get("exact").and_then(Value::as_bool), Some(false));
}

#[test]
fn constructed_missing_stat_path_plan_preserves_explicit_full_path_check() {
    let root = TempDirGuard::new("explicit_missing_stat_path");
    fs::create_dir_all(root.path.join("locator_smart/case_only")).expect("create case dir");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/Report.MD"]
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "检查 locator_smart/Report.MD 是否存在",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!(["locator_smart/Report.MD"])));
}

#[test]
fn structured_scalar_compare_replaces_single_file_read_with_explicit_multi_file_path_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": "/home/guagua/rustclaw/README.md",
            "include_metadata": true
        }),
    }];
    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "compare README.md and AGENTS.md by size, then answer in one sentence",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(!normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, .. } if skill == "doc_parse"
    )));
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths").and_then(Value::as_array).is_some_and(|paths| {
                    paths.iter().any(|value| value.as_str() == Some("README.md"))
                        && paths.iter().any(|value| value.as_str() == Some("AGENTS.md"))
                })
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
}

#[test]
fn structured_task_contract_targets_drive_multi_file_metadata_plan() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": "README.md",
            "include_metadata": true
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "compare these two targets by file metadata",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths").and_then(Value::as_array).is_some_and(|paths| {
                    paths.iter().any(|value| value.as_str() == Some("README.md"))
                        && paths.iter().any(|value| value.as_str() == Some("AGENTS.md"))
                })
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
}

#[test]
fn content_evidence_synthesize_only_plan_reads_structural_file_targets_first() {
    let temp = TempDirGuard::new("content_evidence_multi_read");
    let first = temp.path.join("first.md");
    let second = temp.path.join("second.md");
    fs::write(&first, "first file\nalpha\n").expect("write first file");
    fs::write(&second, "second file\nbeta\n").expect("write second file");

    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.requires_content_evidence = true;
    let noisy_result_path = temp.path.join("mentioned_inside_result.toml");
    fs::write(&noisy_result_path, "ignored = true\n").expect("write noisy result file");
    let plan_context = format!(
        "### RECENT_EXECUTION_EVENTS\n\
             - ts=2 kind=ask request=read {} result=mentions {}\n\
             - ts=1 kind=ask request=read {} result=ok\n\n\
             Direct answer gate resolved execution intent:\n\
             compare the file before last and last file",
        second.display(),
        noisy_result_path.display(),
        first.display()
    );
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "compare the two previously referenced files in one sentence",
        None,
        Some(&plan_context),
        None,
        actions,
    );

    assert_eq!(normalized.len(), 4);
    let first_args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    let first_expected = first.display().to_string();
    assert_eq!(
        first_args.get("path").and_then(Value::as_str),
        Some(first_expected.as_str())
    );
    let second_args = expect_planned_call(&normalized[1], "fs_basic", "read_text_range");
    let second_expected = second.display().to_string();
    assert_eq!(
        second_args.get("path").and_then(Value::as_str),
        Some(second_expected.as_str())
    );
    assert!(matches!(
        normalized.get(2),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn existence_multi_file_stat_paths_are_repaired_from_structural_targets() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": ["README.md", "-CN.md", "Cargo.toml", "no_such_file_20260513.txt"],
            "include_missing": true
        }),
    }];

    let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "检查 README.md, README.zh-CN.md, Cargo.toml, and no_such_file_20260513.txt 是否存在，并用表格返回结果。",
            None,
            actions,
        );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths").and_then(Value::as_array).is_some_and(|paths| {
                    paths.iter().any(|value| value.as_str() == Some("README.md"))
                        && paths.iter().any(|value| value.as_str() == Some("README.zh-CN.md"))
                        && paths.iter().any(|value| value.as_str() == Some("Cargo.toml"))
                        && paths.iter().any(|value| value.as_str() == Some("no_such_file_20260513.txt"))
                        && !paths.iter().any(|value| value.as_str() == Some("-CN.md"))
                })
    ));
}

#[test]
fn explicit_multi_file_metadata_plan_is_not_duplicated_when_targets_are_covered() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "path_batch_facts",
            "paths": ["README.md", "AGENTS.md"]
        }),
    }];
    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "比较 README.md 和 AGENTS.md 的大小，并用一句话解释",
        None,
        actions,
    );

    assert_eq!(
        normalized
            .iter()
            .filter(|action| planned_call_is(action, "fs_basic", "stat_paths"))
            .count(),
        1
    );
}

#[test]
fn normalization_order_schema_aliases_before_multi_target_coverage() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "path_batch_facts",
            "path_list": ["README.md", "AGENTS.md"]
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "compare the two task-contract targets by file metadata",
        None,
        actions,
    );

    let path_fact_actions = normalized
        .iter()
        .filter_map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "fs_basic" => Some(args),
            AgentAction::CallTool { tool, args } if tool == "fs_basic" => Some(args),
            _ => None,
        })
        .filter(|args| args.get("action").and_then(Value::as_str) == Some("stat_paths"))
        .collect::<Vec<_>>();
    assert_eq!(path_fact_actions.len(), 1);
    let args = path_fact_actions[0];
    assert!(args.get("path_list").is_none());
    assert!(args
        .get("paths")
        .and_then(Value::as_array)
        .is_some_and(|paths| {
            paths
                .iter()
                .any(|value| value.as_str() == Some("README.md"))
                && paths
                    .iter()
                    .any(|value| value.as_str() == Some("AGENTS.md"))
        }));
}

#[test]
fn multi_file_modified_time_compare_uses_metadata_not_whole_file_reads() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({"path": "README.md"}),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({"path": "AGENTS.md"}),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "比较这两个文件哪个修改时间更新",
        None,
        actions,
    );

    assert!(!normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, .. } if skill == "read_file"
    )));
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("fields").and_then(Value::as_array).is_some_and(|fields| {
                    fields.iter().any(|value| value.as_str() == Some("modified"))
                })
    ));
}

#[test]
fn recent_scalar_equality_preserves_content_extract_plan_for_explicit_files() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": "Cargo.toml",
                "field_path": "package.version"
            }),
        },
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: serde_json::json!({
                "action": "grep_text",
                "root": "README.md",
                "query": "version",
                "max_matches": 5
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s0".to_string(), "s1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "Read workspace package version from Cargo.toml and compare it with the version mentioned in README.md",
            None,
            actions,
        );

    assert!(!normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("path_batch_facts")
    )));
    assert!(matches!(
        normalized.first(),
        Some(AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args })
            if skill == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("field_path").and_then(Value::as_str) == Some("package.version")
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args })
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("grep_text")
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn recent_scalar_file_pair_plan_reads_structured_field_and_text_evidence() {
    let root = TempDirGuard::new("recent_scalar_file_pair");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write cargo manifest");
    fs::write(
        root.path.join("README.md"),
        "RustClaw release notes\nversion: 0.1.7\n",
    )
    .expect("write readme");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let cargo_path = root.path.join("Cargo.toml").display().to_string();
    let readme_path = root.path.join("README.md").display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{cargo_path} | {readme_path}");
    route.resolved_intent =
        "Read workspace package version from Cargo.toml and compare it with README.md.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = super::recent_scalar_file_pair_deterministic_plan_result(
        &state,
        "Read workspace package version from Cargo.toml and compare it with README.md.",
        Some(&route),
        &loop_state,
        "Read workspace package version from Cargo.toml and compare it with README.md.",
        Some("Read workspace package version from Cargo.toml and compare it with README.md."),
        Some(cargo_path.as_str()),
    )
    .expect("file-pair scalar comparison should use deterministic read plan");

    let first = plan.steps[0]
        .to_agent_action()
        .expect("first step should be an action");
    let args = expect_planned_call(&first, "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(cargo_path.as_str())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );

    let second = plan.steps[1]
        .to_agent_action()
        .expect("second step should be an action");
    let args = expect_planned_call(&second, "fs_basic", "grep_text");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(readme_path.as_str())
    );
    assert_eq!(args.get("query").and_then(Value::as_str), Some("version"));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &plan
            .steps
            .iter()
            .filter_map(|step| step.to_agent_action())
            .collect::<Vec<_>>()
    ));
}

#[test]
fn quantity_compare_preserves_scalar_plus_text_evidence_for_explicit_files() {
    let root = TempDirGuard::new("quantity_scalar_plus_text");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace cargo");
    fs::write(root.path.join("README.md"), "RustClaw v0.1.7\n").expect("write readme");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let cargo_path = root.path.join("Cargo.toml");
    let readme_path = root.path.join("README.md");
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": cargo_path.display().to_string(),
                "field_path": "package.version"
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({
                "path": readme_path.display().to_string()
            }),
        },
    ];
    let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            "Read workspace package version from Cargo.toml and compare it with the version mentioned in README.md",
            Some(cargo_path.to_string_lossy().as_ref()),
            actions,
        );

    assert!(!normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("path_batch_facts")
    )));
    assert!(matches!(
        normalized.first(),
        Some(AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args })
            if skill == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str)
                    == Some(cargo_path.to_string_lossy().as_ref())
                && args.get("field_path").and_then(Value::as_str)
                    == Some("workspace.package.version")
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args })
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str)
                    == Some(readme_path.to_string_lossy().as_ref())
    ));
    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_accepts_compare_paths_for_file_metadata() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "left_path": "Cargo.lock",
            "right_path": "Cargo.toml"
        }),
    }];
    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "比较 Cargo.lock 和 Cargo.toml 的大小",
        None,
        actions,
    );

    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn observation_only_terminal_answer_appends_synthesis_for_builtin_observation() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "logs",
            "files_only": true,
            "sort_by": "mtime_desc",
            "max_entries": 2
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "列出 logs 最近修改的 2 个文件名，并判断更像运行日志还是测试残留",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(planned_call_is(&normalized[0], "fs_basic", "list_dir"));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn observation_only_terminal_answer_keeps_config_basic_scalar_finalizer() {
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "read_field",
            "path": "configs/skills_registry.toml",
            "field_path": "run_cmd.planner_kind"
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_hint = "configs/skills_registry.toml".to_string();

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "在 configs/skills_registry.toml 里找到 run_cmd 的 planner_kind",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_field")
    ));
}

#[test]
fn content_evidence_doc_parse_observation_appends_synthesis() {
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": "release_checklist.md",
            "max_chars": 12000
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "release_checklist.md".to_string();

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "读一下 release_checklist.md，然后一句话告诉我最先该做什么",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn terminal_synthesize_answer_appends_delivery_respond() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["missing.md"],
                "include_missing": true
            }),
        },
        AgentAction::CallSkill {
            skill: "doc_parse".to_string(),
            args: serde_json::json!({
                "action": "extract_key_points",
                "path": "README.md"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
    ];

    let normalized = super::append_respond_for_terminal_synthesize_answer(actions);

    assert_eq!(normalized.len(), 4);
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn observed_terminal_synthesis_replaces_concrete_respond_with_placeholder() {
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "first sentence. second sentence.".to_string(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[0],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn observed_terminal_synthesis_keeps_service_status_concrete_respond() {
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "clawd running; clawd_log.keyword_error_count=43".to_string(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content }
            if content == "clawd running; clawd_log.keyword_error_count=43"
    ));
}

#[test]
fn observed_terminal_synthesis_keeps_structurally_grounded_concrete_respond() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "tree_summary",
                "tree": {
                    "children": [
                        {
                            "kind": "file",
                            "path": "prompts/schemas/intent_normalizer.schema.json",
                            "size_bytes": 13160
                        }
                    ]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let answer =
        "intent_normalizer.schema.json 最大（13160 字节），描述用户意图解析输出。".to_string();
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["round:2.step:3".to_string()],
        },
        AgentAction::Respond {
            content: answer.clone(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &answer
    ));
}

#[test]
fn observed_terminal_synthesis_keeps_identifier_grounded_summary_respond() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Device Local Fixture\n2|\n3|This directory contains stable local files for RustClaw NL regression tests.",
                "path": "/tmp/README.md"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let answer = "该目录为 RustClaw NL 回归测试提供稳定的本地文件样本。".to_string();
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: answer.clone(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &answer
    ));
}

#[test]
fn observation_only_terminal_answer_keeps_file_names_runtime_finalizer() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "logs",
            "files_only": true,
            "sort_by": "mtime_desc",
            "max_entries": 2
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "只输出 logs 最近修改的 2 个文件名",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(planned_call_is(&normalized[0], "fs_basic", "list_dir"));
}

#[test]
fn general_directory_inventory_clears_file_only_filter() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/docs",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "show the directory contents",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn directory_lookup_inventory_clears_file_only_even_with_file_names_semantic() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/docs",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "inspect the directory contents",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn file_names_directory_inventory_preserves_file_only_filter() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/docs",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "output file names only",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn file_names_contract_enforces_file_only_after_find_entries_inventory_rewrite() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": "/workspace/docs"
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "output file names only",
        None,
        actions,
    );

    let Some((tool, args)) = planned_call(&normalized[0]) else {
        panic!("expected fs inventory call, got {:?}", normalized[0]);
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn strict_unclassified_directory_inventory_forces_metadata_for_fs_basic() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": "/workspace/logs",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return the directory listing with requested details",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(1000));
}

#[test]
fn strict_unclassified_system_inventory_forces_metadata_before_fs_rewrite() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/logs",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return the directory listing with requested details",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(1000));
}

#[test]
fn directory_names_contract_enforces_dirs_only_inventory() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace",
            "files_only": true,
            "names_only": false
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "list top-level directory names only",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn directory_names_contract_does_not_invent_dirs_only_without_structured_filter() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": "/workspace/archive",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "list entry names for the resolved directory",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn directory_names_contract_rewrites_filtered_list_dir_to_inventory() {
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: serde_json::json!({
            "path": "/workspace",
            "dirs_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

    let normalized = super::normalize_planned_actions(
        &test_state_with_enabled_skills(&["list_dir", "system_basic"]),
        Some(&route),
        &LoopState::new(2),
        "list top-level directory names only",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert!(args.get("kind_filter").is_none());
}

#[test]
fn list_dir_kind_filter_file_rewrites_to_inventory_file_names() {
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: serde_json::json!({
            "path": "/workspace",
            "kind_filter": "file",
            "limit": 3
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &test_state_with_enabled_skills(&["list_dir", "system_basic"]),
        None,
        &LoopState::new(2),
        "list file names",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(3));
}

#[test]
fn file_paths_contract_rewrites_extension_inventory_to_fs_basic() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": ".",
            "files_only": true,
            "names_only": true,
            "ext_filter": ".toml",
            "max_entries": 5
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return five representative TOML file paths from the repository",
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(args.get("root").and_then(Value::as_str), Some("."));
            assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
            assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
            assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn file_paths_contract_rewrites_fs_basic_list_dir_extension_filter_to_recursive_find() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": "scripts/nl_tests/fixtures/device_local",
            "ext_filter": ".log",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "list matching file paths under a directory",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local")
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("log"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn file_paths_contract_preserves_planned_synthesis_selection() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: serde_json::json!({
                "action": "find_name",
                "root": ".",
                "name": "*.toml",
                "max_results": 50
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return five representative TOML file paths from the repository",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill == "fs_basic"
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn file_paths_anchor_respond_only_adds_find_entries_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    let selected = "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt";
    let plan_context = "\
### ACTIVE_EXECUTION_ANCHOR
followup_bound_target: /home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3
followup_ordered_entries: 1:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md | 2:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt | 3:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt
";
    let actions = vec![AgentAction::Respond {
        content: selected.to_string(),
    }];

    let normalized = super::normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "select the second path",
        None,
        Some(plan_context),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3")
    );
    assert_eq!(
        args.get("pattern").and_then(Value::as_str),
        Some("my_abcd.txt")
    );
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn scalar_path_anchor_respond_only_adds_stat_paths_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    let selected = "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt";
    let plan_context = "\
### ACTIVE_EXECUTION_ANCHOR
followup_bound_target: /home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3
followup_ordered_entries: 1:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md | 2:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt | 3:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt
";
    let actions = vec![AgentAction::Respond {
        content: selected.to_string(),
    }];

    let normalized = super::normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "select the second path",
        None,
        Some(plan_context),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(
        args.get("paths")
            .and_then(Value::as_array)
            .and_then(|items| { items.first().and_then(Value::as_str).map(str::to_string) }),
        Some(selected.to_string())
    );
    assert_eq!(
        args.get("include_missing").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn file_paths_contract_normalizes_fs_search_glob_extension_args() {
    let root = TempDirGuard::new("fs_search_file_paths_contract");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "find_name",
            "basename_pattern": "*.toml",
            "search_root": root_path,
            "type": "file",
            "max_results": 5
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return five representative TOML file paths from the repository",
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("root").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
            assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
        }
        other => panic!("expected normalized fs_basic action, got {other:?}"),
    }
}

#[test]
fn observation_only_terminal_answer_keeps_raw_command_runtime_finalizer() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "pwd" }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "执行 pwd，直接输出命令结果",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
}

#[test]
fn workspace_summary_keeps_requested_structured_field_evidence() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({ "path": "." }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": "UI/package.json",
                "field_path": "name"
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 10
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
            ],
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.resolved_intent =
        "先看顶层目录，再读 UI/package.json 的 name，最后一句话判断 UI 定位".to_string();

    let normalized = super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "config_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_field")
    )));
}

#[test]
fn workspace_summary_with_scope_prunes_sibling_evidence() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({ "path": "UI" }),
        },
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({ "path": "pi_app" }),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_hint = "UI".to_string();
    route.resolved_intent = "Summarize only the UI part of this repository".to_string();

    let pruned = super::prune_unscoped_workspace_summary_evidence_for_scope(
        &test_state(),
        Some(&route),
        actions,
    );
    assert_eq!(pruned.len(), 1);
    match &pruned[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "list_dir");
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("UI")
            );
        }
        other => panic!("expected scoped UI list_dir action, got {other:?}"),
    }
}

#[test]
fn workspace_root_identity_scope_keeps_relative_workspace_evidence() {
    let root = TempDirGuard::new("rustclaw");
    fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap()
        .to_string();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "tree_summary", "path": root.path.display().to_string()}),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "README.md"}),
        },
    ];

    let pruned =
        super::prune_unscoped_workspace_summary_evidence_for_scope(&state, Some(&route), actions);

    assert_eq!(pruned.len(), 2);
    assert!(matches!(
        &pruned[1],
        AgentAction::CallSkill { skill, args }
            if skill == "read_file"
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    ));
}

#[test]
fn unscoped_workspace_evidence_appends_synthesis_after_existing_text_read_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_existing");
    fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({"path":"README.md"}),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );
    assert_eq!(normalized.len(), 3);
    assert!(planned_call_is(
        &normalized[0],
        "fs_basic",
        "read_text_range"
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn unscoped_workspace_text_answer_strips_unrequested_file_artifact_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_no_artifact");
    fs::write(
        root.path.join("README.md"),
        "# RustClaw\n\nUse the documented installer",
    )
    .expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    route.resolved_intent = "Write a short RustClaw setup note".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path":"Cargo.toml"}),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: json!({
                "path":"document/SETUP_NOTE.md",
                "content":"# RustClaw Setup Note\n"
            }),
        },
        AgentAction::Respond {
            content: "FILE:/home/guagua/rustclaw/document/SETUP_NOTE.md".to_string(),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. } if skill == "write_file"
        ) && !planned_call_is(action, "fs_basic", "write_text")
    }));
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::Respond { content } if content.trim().starts_with("FILE:")
        )
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string()]
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn active_execution_recipe_keeps_workspace_file_mutation_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_recipe_mutation");
    fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint.clear();
    let mut loop_state = LoopState::new(1);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "write_file".to_string(),
        args: json!({
            "path":"document/SETUP_NOTE.md",
            "content":"# RustClaw Setup Note\n"
        }),
    }];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
}

#[test]
fn explicit_workspace_file_locator_keeps_requested_file_mutation_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_requested_mutation");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "plan/p2_expand_test.md".to_string();
    route.resolved_intent = "Create plan/p2_expand_test.md and write p2 hello".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "make_dir".to_string(),
            args: json!({"path":"plan"}),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: json!({
                "path":"plan/p2_expand_test.md",
                "content":"p2 hello"
            }),
        },
        AgentAction::Respond {
            content: "created".to_string(),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "make_dir") }));
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
}

#[test]
fn delivery_write_strips_redundant_make_dir_and_appends_file_token() {
    let root = TempDirGuard::new("delivery_write_generic");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::FileToken,
    );
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document/manual_meta.json".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.wants_file_delivery = true;
    route.resolved_intent =
        "Generate document/manual_meta.json and send the file to the user.".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "make_dir",
                "path": "document"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": "document/manual_meta.json",
                "content": "{\"app\":\"RustClaw\",\"test\":\"nl\"}"
            }),
        },
    ];

    let normalized = super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );

    assert!(!normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "make_dir") }));
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
    let expected = format!("FILE:{}/document/manual_meta.json", root.path.display());
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == &expected
    ));
}

#[test]
fn free_route_strips_terminal_discussion_after_runner_skill() {
    let state = test_state();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "crypto".to_string(),
            args: serde_json::json!({ "action": "quote", "symbol": "BTCUSDT" }),
        },
        AgentAction::Respond {
            content: "下面是我帮你整理后的结果。".to_string(),
        },
    ];

    let stripped = strip_terminal_discussion_for_direct_skill_passthrough(
        &state,
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        actions,
    );
    assert_eq!(stripped.len(), 1);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, .. } if skill == "crypto"
    ));
}

#[test]
fn process_basic_port_list_keeps_terminal_discussion_followup() {
    let state = test_state();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({ "action": "port_list" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let kept = strip_terminal_discussion_for_direct_skill_passthrough(
        &state,
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::Free,
        )),
        actions.clone(),
    );
    assert_eq!(kept.len(), 3);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, args }
            if skill == "process_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("port_list")
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &kept[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn service_status_process_basic_port_list_strips_terminal_synthesis() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({ "action": "port_list" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let stripped =
        strip_terminal_discussion_for_direct_skill_passthrough(&state, Some(&route), actions);

    assert_eq!(stripped.len(), 1);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, args }
            if skill == "process_basic"
                && args.get("action").and_then(Value::as_str) == Some("port_list")
    ));
}

#[test]
fn process_basic_synthesis_survives_workspace_text_guard_for_exact_sentence() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({ "action": "port_list" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "process_basic"
                && args.get("action").and_then(Value::as_str) == Some("port_list")
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn output_template_code_span_is_not_treated_as_literal_command() {
    let request = "Read Cargo.toml version and answer as `version=<value>` only.";
    assert!(super::shellish_literal_command_segment(request).is_none());

    let state = test_state_with_enabled_skills(&["run_cmd", "config_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "Cargo.toml".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    assert!(explicit_command_deterministic_plan_result(
        &state,
        "<goal>",
        Some(&route),
        &loop_state,
        request,
    )
    .is_none());
}

#[test]
fn colon_output_template_code_span_is_not_treated_as_literal_command() {
    let request = "Return the current git branch in the format `branch: NAME`.";
    assert!(super::shellish_literal_command_segment(request).is_none());

    let state = test_state_with_enabled_skills(&["run_cmd", "git_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;

    assert!(explicit_command_deterministic_plan_result(
        &state,
        "<goal>",
        Some(&route),
        &loop_state,
        request,
    )
    .is_none());
}

#[test]
fn concrete_shell_code_span_still_uses_literal_command_path() {
    let request = "Check current directory with `pwd && ls Cargo.toml`.";
    assert_eq!(
        super::shellish_literal_command_segment(request).as_deref(),
        Some("pwd && ls Cargo.toml")
    );
}

#[test]
fn direct_passthrough_keeps_mixed_placeholder_terminal_respond() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "pwd" }),
        },
        AgentAction::Respond {
            content: "{{last_output}}\n\nworkspace ready".to_string(),
        },
    ];

    let kept =
        strip_terminal_discussion_for_direct_skill_passthrough(&state, Some(&route), actions);
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::Respond { content } if content.contains("workspace ready")
    ));
}

#[test]
fn strict_run_cmd_template_preserves_mixed_last_output_respond() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "pwd" }),
        },
        AgentAction::Respond {
            content: "cwd={{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "运行 pwd 命令，并按 key=value 模板返回当前目录。",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert_eq!(
        actions_as_json(&normalized),
        json!([
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": { "command": "pwd" }
            },
            {
                "type": "respond",
                "content": "cwd={{last_output}}"
            }
        ])
    );
}

#[test]
fn runner_skill_only_plan_does_not_require_terminal_respond() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "crypto".to_string(),
        args: serde_json::json!({ "action": "quote", "symbol": "BTCUSDT" }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn chat_wrapped_execution_route_repairs_observation_only_plan_before_any_observation() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn chat_wrapped_execution_route_repairs_observation_plus_unavailable_followup_plan() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        },
        AgentAction::CallSkill {
            skill: "formatter".to_string(),
            args: serde_json::json!({ "text": "explain {{last_output}}" }),
        },
    ];
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        false,
        OutputResponseShape::Free,
    );
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "unavailable_skill_requires_replan"
    );
}

#[test]
fn chat_wrapped_execution_route_keeps_observation_plus_synthesize_followup_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn chat_wrapped_execution_route_keeps_health_check_observation_only_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "health_check".to_string(),
        args: serde_json::json!({}),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::OneSentence,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn non_scalar_route_still_repairs_after_prior_observation_when_delivery_is_empty() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn scalar_route_keeps_single_observation_plan_without_followup() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "current_branch" }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Scalar,
    );
    assert!(
        !should_force_plan_repair(Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        repair_reason(Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn git_basic_branch_alias_scalar_route_normalizes_to_current_branch() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    let actions = vec![AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "branches" }),
    }];

    let normalized = normalize_git_basic_schema_aliases(Some(&route), actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("current_branch")
    ));
}

#[test]
fn git_basic_branch_alias_non_scalar_route_normalizes_to_branch() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let actions = vec![AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "branches" }),
    }];

    let normalized = normalize_git_basic_schema_aliases(Some(&route), actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("branch")
    ));
}

#[test]
fn git_repository_state_remote_request_plans_git_remote_action() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::GitRepositoryState;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    let plan = git_repository_state_deterministic_plan_result(
        "列出当前仓库 remote 名称和 URL",
        Some(&route),
        &loop_state,
        "列出当前仓库 remote 名称和 URL",
    )
    .expect("git repository state plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "git_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("remote")
    );
}

#[test]
fn git_repository_state_contract_defaults_to_status_without_nl_matching() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::GitRepositoryState;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    let plan = git_repository_state_deterministic_plan_result(
        "semantic contract only",
        Some(&route),
        &loop_state,
        "检查这个仓库当前是否有未提交改动，用一句话说明。",
    )
    .expect("git repository state plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "git_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("status")
    );
}

#[test]
fn recent_scalar_current_workspace_plans_git_branch_without_nl_matching() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    let plan = super::recent_scalar_current_workspace_deterministic_plan_result(
        &state,
        "semantic contract only",
        Some(&route),
        &loop_state,
    )
    .expect("recent scalar current workspace plan");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].skill, "git_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("current_branch")
    );
}

#[test]
fn recent_scalar_current_workspace_git_observation_satisfies_repair_guard() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({ "action": "current_branch" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn raw_command_output_route_keeps_single_run_cmd_plan_without_followup() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "ls", "cwd": "/tmp/rustclaw-workspace" }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn runtime_status_scalar_patch_plans_current_user_system_basic_status() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "runtime_status_query": {"kind": "current_user", "scope": "system"}
        })),
        attachment_processing_required: false,
    };

    let plan = super::runtime_status_scalar_deterministic_plan_result(
        &state,
        "return current user",
        Some(&route),
        &loop_state,
        Some(&analysis),
    )
    .expect("runtime status patch should produce deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "system_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(
        plan.steps[0].args.get("kind").and_then(Value::as_str),
        Some("current_user")
    );
}

#[test]
fn runtime_status_scalar_tool_marker_without_kind_defers_to_planner() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.route_reason = "execution_recipe_scalar_runtime_tool_observation".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(super::runtime_status_scalar_info_fallback_plan_result(
        &state,
        "return runtime scalar",
        Some(&route),
        &loop_state,
        Some(&analysis),
    )
    .is_none());
}

#[test]
fn file_delivery_route_allows_plain_not_found_terminal_reply() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "未找到该文件。".to_string(),
    }];
    assert!(!should_force_plan_repair(
        Some(&delivery_route_result()),
        &loop_state,
        &actions,
    ));
}

#[test]
fn ops_recipe_apply_phase_without_mutation_forces_plan_repair() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "http_basic".to_string(),
        args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:60703/" }),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn ops_recipe_apply_phase_without_mutation_uses_specific_repair_reason() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "http_basic".to_string(),
        args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:60703/" }),
    }];
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "ops_closed_loop_apply_requires_mutation"
    );
}

#[test]
fn ops_recipe_apply_phase_with_mutation_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "document/index.html" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "document/index.html", "content": "ops-repair-ok\n" }),
        },
    ];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn config_change_profile_without_post_change_validation_forces_repair() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "configs/config.toml" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "configs/config.toml", "content": "[tools]\nallow_sudo=false\n" }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "config_change_requires_post_change_validation"
    );
}

#[test]
fn skill_authoring_profile_requires_integration_validation_not_readback_only() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/skills/foo/INTERFACE.md" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/skills/foo/INTERFACE.md", "content": "# Foo\n" }),
        },
        AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:62078/" }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "skill_authoring_requires_integration_validation"
    );
}

#[test]
fn code_change_profile_requires_verification_not_readback_only() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs" }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "code_change_requires_verification"
    );
}

#[test]
fn code_change_profile_with_structured_cargo_check_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "clawd"
                }
            }),
        },
    ];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn code_change_profile_with_unstructured_cargo_check_forces_repair() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "cargo check -p clawd" }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            Some(&actions),
        ),
        "code_change_requires_verification"
    );
}

#[test]
fn current_repo_scope_rejects_external_absolute_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "/opt/other-project/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "tools/demo"
                }
            }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "current_repo_scope_rejects_external_target"
    );
}

#[test]
fn external_workspace_scope_requires_explicit_external_target() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "tools/demo"
                }
            }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "external_workspace_requires_explicit_target"
    );
}

#[test]
fn greenfield_scope_requires_creation_step_before_validation() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "cargo check -p clawd" }),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::planner_execute_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "greenfield_requires_artifact_creation"
    );
}

#[test]
fn greenfield_scope_with_make_dir_and_write_file_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "make_dir".to_string(),
            args: serde_json::json!({ "path": "tools/demo" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "tools/demo/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "tools/demo"
                }
            }),
        },
    ];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Scalar,
    );
    assert!(
        !should_force_plan_repair(Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        repair_reason(Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn external_workspace_scope_persists_across_rounds_without_repeating_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_external_target: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({
            "command": "cargo check",
            "_clawd_validation": {
                "profile": "code_change",
                "validator_type": "build",
                "validated_target": "external_workspace"
            }
        }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn greenfield_scope_persists_creation_across_rounds() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_greenfield_creation: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({
            "command": "cargo check -p clawd",
            "_clawd_validation": {
                "profile": "code_change",
                "validator_type": "build",
                "validated_target": "greenfield_project"
            }
        }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_evidence_route_allows_respond_only_after_prior_observation() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "grounded final answer".to_string(),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn extracts_xml_call_skill_markup_into_step_values() {
    let raw = r#"<tool_call>
<invoke name="call_skill">
<parameter name="skill">list_dir</parameter>
<parameter name="args">{"path": "/tmp"}</parameter>
</invoke>
</tool_call>"#;
    assert_eq!(
        super::extract_xml_tool_call_steps(raw),
        vec![json!({
            "type": "call_skill",
            "skill": "list_dir",
            "args": { "path": "/tmp" }
        })]
    );
}

#[test]
fn extracts_xml_direct_skill_invoke_markup_into_step_values() {
    let raw = r#"<tool_call>
<invoke name="fs_search">
<parameter name="args">{"action":"find_name","pattern":"README"}</parameter>
</invoke>
</tool_call>"#;
    assert_eq!(
        super::extract_xml_tool_call_steps(raw),
        vec![json!({
            "type": "call_skill",
            "skill": "fs_search",
            "args": { "action": "find_name", "pattern": "README" }
        })]
    );
}

// ---------- inject_synthesize_answer_for_bare_placeholder_respond ----------
// 见函数 doc：runtime 兜底，把兼容模型偶发吐出的裸 placeholder respond 注入
// 一个 synthesize_answer 节点，关掉裸 placeholder 导致的死循环。

#[test]
fn strips_intermediate_synthesize_before_later_execution() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "path_batch_facts", "paths": ["missing.txt"]}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "scripts"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let stripped = strip_intermediate_synthesize_before_later_execution(actions);

    assert_eq!(stripped.len(), 3);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &stripped[1],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &stripped[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn strips_terminal_placeholder_respond_for_exact_listing_contract() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "scripts"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let stripped =
        strip_terminal_placeholder_respond_for_exact_listing_contract(Some(&route), actions);

    assert_eq!(stripped.len(), 1);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
}

#[test]
fn detects_bare_last_output_placeholder_variants() {
    assert!(is_bare_last_output_placeholder("{{last_output}}"));
    assert!(is_bare_last_output_placeholder("  {{ last_output }}  "));
    assert!(is_bare_last_output_placeholder("{{last_output.hostname}}"));
    assert!(is_bare_last_output_placeholder("{{last_output.foo.bar}}"));
    assert!(is_bare_last_output_placeholder("{{LAST_OUTPUT}}"));
    assert!(is_bare_last_output_placeholder("{{last_output[\"x\"]}}"));
}

#[test]
fn rejects_non_bare_placeholder_content() {
    assert!(!is_bare_last_output_placeholder(
        "hostname is {{last_output}}"
    ));
    assert!(!is_bare_last_output_placeholder("当前用户是 root"));
    assert!(!is_bare_last_output_placeholder(""));
    assert!(!is_bare_last_output_placeholder("{{other}}"));
    assert!(!is_bare_last_output_placeholder("{{lastoutput}}"));
    // last_output 后接非 . / [ 的字符不算同一占位
    assert!(!is_bare_last_output_placeholder("{{last_output_extra}}"));
}

#[test]
fn injects_synthesize_answer_when_respond_is_bare_placeholder() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "whoami" }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let out = inject_synthesize_answer_for_bare_placeholder_respond(
        actions,
        "只输出当前用户名，不要解释",
    );
    assert_eq!(out.len(), 3, "should insert exactly one synth step");
    assert!(matches!(
        &out[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    match &out[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(
                evidence_refs,
                &vec!["last_output".to_string()],
                "synthesize step should point at last_output by default"
            );
        }
        _ => panic!("expected synthesize_answer at index 1, got {:?}", out[1]),
    }
    assert!(matches!(
        &out[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

fn actions_as_json(actions: &[AgentAction]) -> serde_json::Value {
    serde_json::to_value(actions).expect("serialize")
}

#[test]
fn injection_is_idempotent_when_synthesize_already_precedes_respond() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "whoami" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(
        actions_as_json(&out),
        before,
        "should not re-inject when synthesize_answer already precedes respond"
    );
}

#[test]
fn terminal_synthesis_placeholder_respond_uses_last_output() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": "README.md" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{synthesized}}".to_string(),
        },
    ];

    let out = rewrite_terminal_synthesis_placeholder_respond(actions);
    assert_eq!(out.len(), 3);
    assert!(matches!(
        &out[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn injection_no_op_when_respond_content_is_concrete() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "whoami" }),
        },
        AgentAction::Respond {
            content: "guagua".to_string(),
        },
    ];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(actions_as_json(&out), before);
}

#[test]
fn injection_no_op_when_only_one_action() {
    let actions = vec![AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    }];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(
        actions_as_json(&out),
        before,
        "no observation step before respond → cannot meaningfully inject"
    );
}

#[test]
fn injection_no_op_when_last_action_is_not_respond() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({ "command": "ls" }),
    }];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(actions_as_json(&out), before);
}

#[test]
fn normalizer_drops_pre_observation_synthesize_when_concrete_respond_exists() {
    let state = test_state();
    let loop_state = LoopState::new(2);
    let route = route_result(
        crate::AskMode::direct_answer(),
        false,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "早出晚归皆是梦，\n一杯咖啡换人间。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "写一首两句的打工人短诗",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::Respond { content }
            if content == "早出晚归皆是梦，\n一杯咖啡换人间。"
    ));
}

#[test]
fn normalizer_keeps_prior_observation_synthesize_and_placeholders_concrete_respond() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some("{\"ports_snapshot\":[\"0.0.0.0:22\"]}".to_string());
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "监听端口里最值得注意的是 0.0.0.0:22。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "看看这台机器现在有哪些端口在监听",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        &normalized[0],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn normalizer_keeps_observation_backed_synthesize_before_respond() {
    let state = test_state();
    let loop_state = LoopState::new(2);
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "pwd" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let before = actions_as_json(&actions);

    let normalized =
        normalize_planned_actions(&state, Some(&route), &loop_state, "执行 pwd", None, actions);

    assert_eq!(actions_as_json(&normalized), before);
}

/// §F1：`has_pre_observation_structured_output_shape` 结构形态检测覆盖。
#[test]
fn pre_observation_structured_output_shape_recognizes_listing_shapes() {
    // 真实 adv08 复现：list_dir 还没跑，respond 编出 5 行 numbered 列表 + 路径。
    let adv08 = "prompts 目录前 5 个文件名：\n1. prompts/skills\n2. prompts/agents\n3. prompts/system\n4. prompts/user\n5. prompts/layers";
    assert!(has_pre_observation_structured_output_shape(adv08));

    // 多行 + 文件后缀，但没编号。
    let multi_paths = "Cargo.toml\nCargo.lock\nREADME.md\nLICENSE";
    assert!(has_pre_observation_structured_output_shape(multi_paths));

    // 结构化字段标签。
    assert!(has_pre_observation_structured_output_shape(
        "result: 42\ncount: 3"
    ));

    // 一句正常文本 → 不命中。
    assert!(!has_pre_observation_structured_output_shape(
        "好的，正在查询，请稍候。"
    ));
    // {{last_output}} 占位符 → 不命中（应由 synthesize 注入兜底处理）。
    assert!(!has_pre_observation_structured_output_shape(
        "{{last_output}}"
    ));
    // 只有一行短回复 → 不命中。
    assert!(!has_pre_observation_structured_output_shape("yes"));
}

/// §F1：rewrite 触发条件 —— round 1 + 上一步 CallSkill + Respond 含枚举。
#[test]
fn rewrite_pre_observation_rewrites_concrete_respond_after_call_skill() {
    let loop_state = LoopState::new(2);
    assert!(loop_state.executed_step_results.is_empty());
    assert!(loop_state.last_output.is_none());

    let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path": "/home/guagua/rustclaw/prompts"}),
            },
            AgentAction::Respond {
                content: "prompts 目录前 5 个文件名：\n1. prompts/skills\n2. prompts/agents\n3. prompts/system\n4. prompts/user\n5. prompts/layers".to_string(),
            },
        ];
    let out = rewrite_pre_observation_concrete_respond_to_placeholder(None, &loop_state, actions);
    match out.last().expect("should have a last action") {
        AgentAction::Respond { content } => {
            assert_eq!(
                content, "{{last_output}}",
                "concrete content must be replaced with placeholder"
            );
        }
        other => panic!("last action should remain Respond, got: {:?}", other),
    }
}

#[test]
fn rewrite_pre_observation_uses_output_contract_without_shape_matching() {
    let loop_state = LoopState::new(2);
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args: json!({"action": "status", "service": "rustclaw"}),
        },
        AgentAction::Respond {
            content: "服务运行正常，可以继续使用。".to_string(),
        },
    ];

    let out =
        rewrite_pre_observation_concrete_respond_to_placeholder(Some(&route), &loop_state, actions);

    assert!(matches!(
        out.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

/// §F1：执行过任何 step 后不再触发（避免误改 round 2+ 的合法 grounded respond）。
#[test]
fn rewrite_pre_observation_no_op_after_any_step_executed() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "s1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("foo\nbar".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_output = Some("foo\nbar".to_string());

    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path": "/x"}),
        },
        AgentAction::Respond {
            content: "1. foo\n2. bar".to_string(),
        },
    ];
    let before = actions.clone();
    let after = rewrite_pre_observation_concrete_respond_to_placeholder(None, &loop_state, actions);
    assert_eq!(actions_as_json(&before), actions_as_json(&after));
}

/// §F1：Respond 内容是合法占位符或短确认时不触发。
#[test]
fn rewrite_pre_observation_no_op_for_placeholder_or_short_ack() {
    let loop_state = LoopState::new(2);
    for content in ["{{last_output}}", "好的", "稍候，正在执行"] {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "ls"}),
            },
            AgentAction::Respond {
                content: content.to_string(),
            },
        ];
        let before = actions.clone();
        let after =
            rewrite_pre_observation_concrete_respond_to_placeholder(None, &loop_state, actions);
        assert_eq!(
            actions_as_json(&before),
            actions_as_json(&after),
            "should not rewrite for content={:?}",
            content
        );
    }
}

#[test]
fn rewrite_terminal_placeholder_respond_inserts_synthesize_answer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "service_notes.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": "README.md" }),
        },
        AgentAction::Respond {
            content: "先看 {{s1.output}}，再说明 {{s2.output}} 的作用".to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 4);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::CallSkill { skill, .. } if skill == "read_file"
    ));
    assert!(matches!(
        &rewritten[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice()
                == ["s1.output".to_string(), "s2.output".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn normalized_multi_command_failure_summary_preserves_all_observations() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let loop_state = LoopState::new(1);
    let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "echo THINK_BREAK_CN"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "definitely_missing_command_minimax_think_24690"}),
            },
            AgentAction::Respond {
                content: "执行结果总结：\n\n- **echo THINK_BREAK_CN** -> 成功，输出：{{s1.output}}\n- **definitely_missing_command_minimax_think_24690** -> 失败，输出：{{s2.output}}"
                    .to_string(),
            },
        ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "先执行 echo THINK_BREAK_CN，再执行 definitely_missing_command_minimax_think_24690，然后总结成功和失败分别是什么",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

    assert_eq!(
        actions_as_json(&normalized),
        json!([
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "echo THINK_BREAK_CN",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "definitely_missing_command_minimax_think_24690",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "synthesize_answer",
                "evidence_refs": ["step_1", "step_2"]
            },
            {
                "type": "respond",
                "content": "{{last_output}}"
            }
        ])
    );
    assert_eq!(normalized.len(), 4);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str) == Some("echo THINK_BREAK_CN")
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str)
                    == Some("definitely_missing_command_minimax_think_24690")
    ));
    assert_eq!(
        super::action_args(&normalized[0])
            .and_then(|args| args.get("_clawd_continue_on_error"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        super::action_args(&normalized[1])
            .and_then(|args| args.get("_clawd_continue_on_error"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice()
                == ["step_1".to_string(), "step_2".to_string()].as_slice()
    ));
    assert!(matches!(
        &normalized[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn normalized_run_cmd_observation_sequence_marks_continue_on_error() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "printenv PATH"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "definitely_absent_command_for_sequence_marker"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "uname -s"}),
        },
    ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Run the listed command sequence and report each result.",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert!(normalized.len() >= 3);
    for action in normalized.iter().take(3) {
        let args = super::action_args(action).expect("run_cmd args");
        assert_eq!(
            args.get("_clawd_continue_on_error")
                .and_then(Value::as_bool),
            Some(true)
        );
    }
}

#[test]
fn normalized_run_cmd_mutation_sequence_does_not_mark_continue_on_error() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "mkdir tmp_sequence_marker"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pwd"}),
        },
    ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Run this setup command and then inspect the current directory.",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert!(normalized.len() >= 2);
    for action in normalized.iter().take(2) {
        let args = super::action_args(action).expect("run_cmd args");
        assert_eq!(args.get("_clawd_continue_on_error"), None);
    }
}

#[test]
fn planner_introduced_tail_run_cmd_rewrites_to_fs_basic_read_range() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "tail -n 3 /home/guagua/rustclaw/logs/clawd.run.log"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查看 logs/clawd.run.log 最后 3 行，只做简短概述。",
        None,
        Some("/home/guagua/rustclaw/logs/clawd.run.log"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/logs/clawd.run.log")
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(3));
}

#[test]
fn content_excerpt_summary_tail_run_cmd_does_not_insert_default_head_read() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    let loop_state = LoopState::new(1);
    let path = "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/model_io.log";
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": format!("tail -n 4 {path}")}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "看一下日志最后 4 行，再一句话说有没有失败后恢复。",
        None,
        Some(path),
        actions,
    );

    let reads: Vec<&Value> = normalized
        .iter()
        .filter_map(|action| {
            planned_call(action).and_then(|(tool, args)| {
                (tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_text_range"))
                .then_some(args)
            })
        })
        .collect();
    assert_eq!(reads.len(), 1, "normalized actions: {normalized:?}");
    assert_eq!(reads[0].get("path").and_then(Value::as_str), Some(path));
    assert_eq!(reads[0].get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(reads[0].get("n").and_then(Value::as_u64), Some(4));
}

#[test]
fn planner_introduced_echo_append_run_cmd_rewrites_to_fs_basic_append_text() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallTool {
        tool: "run_cmd".to_string(),
        args: json!({
            "command": "echo \"beta\" >> document/nl_tool200/group_02/memo.txt",
            "cwd": "/home/guagua/rustclaw"
        }),
    }];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Append beta to the known memo file.",
        None,
        Some("/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "append_text");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt")
    );
    assert_eq!(args.get("content").and_then(Value::as_str), Some("beta\n"));
}

#[test]
fn user_supplied_tail_command_stays_run_cmd() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let command = "tail -n 3 logs/clawd.run.log";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "执行 tail -n 3 logs/clawd.run.log",
        Some("执行 tail -n 3 logs/clawd.run.log"),
        Some("/home/guagua/rustclaw/logs/clawd.run.log"),
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected preserved run_cmd action, got {other:?}"),
    }
}

#[test]
fn planner_introduced_find_extension_dirs_rewrites_to_fs_basic() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let command = r#"find . -name '*.sh' -type f -exec dirname {} \; | sort -u"#;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": command}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(args.get("root").and_then(Value::as_str), Some("."));
    assert_eq!(args.get("extension").and_then(Value::as_str), Some("sh"));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn planner_introduced_find_sed_parent_dirs_rewrites_to_fs_basic() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let command =
        r#"find /home/guagua/rustclaw -name '*.sh' -type f | sed 's|/[^/]*$||' | sort -u"#;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "扫描当前仓库中所有.sh文件，提取其所在目录路径并去重排序后输出",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("/home/guagua/rustclaw")
    );
    assert_eq!(args.get("extension").and_then(Value::as_str), Some("sh"));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn structured_find_observation_strips_redundant_shell_followup() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "root": "/home/guagua/rustclaw",
                "ext": "sh",
                "target_kind": "file"
            }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "find /home/guagua/rustclaw -name '*.sh' -exec dirname {} \\; | sort -u"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert!(normalized.iter().all(
        |action| !matches!(action, AgentAction::CallSkill { skill, .. } if skill == "run_cmd")
    ));
    assert!(normalized
        .iter()
        .all(|action| !matches!(action, AgentAction::SynthesizeAnswer { .. })));
    assert!(normalized
        .iter()
        .all(|action| planned_call_is(action, "fs_basic", "find_entries")));
}

#[test]
fn user_supplied_find_extension_command_stays_run_cmd() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let command = r#"find . -name '*.sh' -type f -exec dirname {} \; | sort -u"#;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "执行 find . -name '*.sh' -type f -exec dirname {} \\; | sort -u",
        Some("执行 find . -name '*.sh' -type f -exec dirname {} \\; | sort -u"),
        Some("/home/guagua/rustclaw"),
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected preserved run_cmd action, got {other:?}"),
    }
}

#[test]
fn piped_tail_command_is_not_rewritten_to_file_tool() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let command = "tail -n 3 logs/clawd.run.log | sed -n '1p'";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查看日志尾部第一行",
        None,
        Some("/home/guagua/rustclaw/logs/clawd.run.log"),
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected preserved piped run_cmd action, got {other:?}"),
    }
}

#[test]
fn normalized_single_sequential_run_cmd_splits_for_step_status_evidence() {
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "echo THINK_BREAK_CN; definitely_missing_command_minimax_think_24690",
            "cwd": "/home/guagua/rustclaw"
        }),
    }];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行两个命令：echo THINK_BREAK_CN 和 definitely_missing_command_minimax_think_24690，然后总结哪些成功、哪些失败",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

    assert_eq!(
        actions_as_json(&normalized),
        json!([
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "echo THINK_BREAK_CN",
                    "cwd": "/home/guagua/rustclaw",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "definitely_missing_command_minimax_think_24690",
                    "cwd": "/home/guagua/rustclaw",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "synthesize_answer",
                "evidence_refs": ["step_1", "step_2"]
            },
            {
                "type": "respond",
                "content": "{{last_output}}"
            }
        ])
    );
}

#[test]
fn normalized_planner_introduced_and_sequence_splits_for_step_status_evidence() {
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "echo BEFORE_BREAK && definitely_missing_command_rustclaw_user_ops_13579"
        }),
    }];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行两个命令：先 echo BEFORE_BREAK，再 definitely_missing_command_rustclaw_user_ops_13579，报告哪一步失败了",
            Some(
                "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
            ),
            Some("/home/guagua/rustclaw"),
            actions,
        );

    assert_eq!(
        actions_as_json(&normalized),
        json!([
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "echo BEFORE_BREAK",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "definitely_missing_command_rustclaw_user_ops_13579",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "synthesize_answer",
                "evidence_refs": ["step_1", "step_2"]
            },
            {
                "type": "respond",
                "content": "{{last_output}}"
            }
        ])
    );
}

#[test]
fn user_supplied_and_operator_is_preserved_as_one_command() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "echo BEFORE_BREAK && echo AFTER_BREAK"}),
    }];

    let rewritten = super::split_sequential_run_cmd_actions(
        "Run `echo BEFORE_BREAK && echo AFTER_BREAK` exactly.",
        Some("Run `echo BEFORE_BREAK && echo AFTER_BREAK` exactly."),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("echo BEFORE_BREAK && echo AFTER_BREAK")
    ));
}

#[test]
fn user_supplied_or_operator_is_preserved_as_one_command() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "missing_probe --version || which bash"}),
    }];

    let rewritten = super::split_sequential_run_cmd_actions(
        "Run `missing_probe --version || which bash` exactly.",
        Some("Run `missing_probe --version || which bash` exactly."),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("missing_probe --version || which bash")
    ));
}

#[test]
fn user_supplied_semicolon_command_is_preserved_as_one_command() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "printf problem >&2; exit 7"}),
    }];

    let rewritten = super::split_sequential_run_cmd_actions(
        "执行命令 `printf problem >&2; exit 7`，报告退出码和 stderr 错误输出。",
        Some("执行命令 printf problem >&2; exit 7，告诉我退出码和错误输出。"),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("printf problem >&2; exit 7")
    ));
}

#[test]
fn planner_introduced_or_operator_becomes_first_visible_attempt() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "missing_probe --version 2>/dev/null || which bash",
            "_clawd_continue_on_error": true,
            "_clawd_literal_command": true
        }),
    }];

    let rewritten = super::split_sequential_run_cmd_actions(
        "Run missing_probe --version. If it is missing, run which bash.",
        Some("Run missing_probe --version. If it is missing, run which bash."),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("missing_probe --version 2>/dev/null")
                && args.get("_clawd_continue_on_error").is_none()
                && args.get("_clawd_literal_command").is_none()
    ));
}

#[test]
fn planner_introduced_and_operator_can_split_when_user_did_not_supply_it() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "echo BEFORE_BREAK && echo AFTER_BREAK"}),
    }];

    let rewritten = super::split_sequential_run_cmd_actions(
        "Run echo BEFORE_BREAK, then run echo AFTER_BREAK.",
        Some("Run echo BEFORE_BREAK, then run echo AFTER_BREAK."),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str) == Some("echo BEFORE_BREAK")
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str) == Some("echo AFTER_BREAK")
    ));
}

#[test]
fn shell_sequence_splitter_ignores_quoted_semicolons_and_stateful_prefixes() {
    assert_eq!(
        super::split_shell_sequence_command_with_policy("echo a; echo b", false),
        Some(vec!["echo a".to_string(), "echo b".to_string()])
    );
    assert_eq!(
        super::split_shell_sequence_command_with_policy("printf 'a;b\\n'", false),
        None
    );
    assert_eq!(
        super::split_shell_sequence_command_with_policy("cd /tmp; pwd", false),
        None
    );
}

#[test]
fn rewrite_terminal_expression_placeholder_respond_inserts_synthesize_answer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "extract_field", "path": "package.json", "field_path": "name"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "extract_field", "path": "Cargo.toml", "field_path": "package.name"}),
        },
        AgentAction::Respond {
            content: "name={{s1}}; crate={{s2}}; same={{s1 == s2 ? 'yes' : 'no'}}".to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 4);
    assert!(matches!(
        &rewritten[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice() == ["s1".to_string(), "s2".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn rewrite_terminal_step_output_alias_placeholder_inserts_synthesize_answer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "docs"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": "docs/release_checklist.md"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{step1_output}} and {{step3_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 5);
    assert!(matches!(
        &rewritten[3],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice() == ["step_1".to_string(), "step_3".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[4],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn rewrite_terminal_placeholder_preserves_mixed_last_output_respond() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pwd"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}\n\n这个路径是当前工作目录，通常对应正在操作的项目根目录。"
                .to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content }
            if content.contains("{{last_output}}") && content.contains("当前工作目录")
    ));
}

#[test]
fn unresolved_template_arg_multi_file_read_plan_uses_direct_file_reads() {
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": "README.md", "mode": "head", "n": 40}),
        },
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({"action": "find_name", "name": "AGENTS.md"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": "{{s1_match}}", "mode": "head", "n": 40}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s0".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_unresolved_template_arg_multi_file_read_plan(
        Some(&route),
        "read the opening section of README.md, then read the opening section of AGENTS.md",
        actions,
    );

    assert_eq!(rewritten.len(), 4);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("AGENTS.md")
    ));
    assert!(matches!(
        &rewritten[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice() == ["step_1".to_string(), "step_2".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

/// §D2.a：plan_result schema 与 `AgentAction` enum / `SinglePlanEnvelope` 漂移检查。
///
/// 校验内容：
/// 1. `prompts/schemas/plan_result.schema.json` 是合法 JSON 且为 object schema；
/// 2. envelope 顶层 required 含 `steps`；
/// 3. `$defs/AgentAction.oneOf` 必须正好覆盖 6 个 variant：think / call_skill /
///    call_tool / call_capability / synthesize_answer / respond（与 `AgentAction` enum 一一对应）；
/// 4. 每个 variant 的 `type` const 必须是 snake_case 的 variant 名；
/// 5. 每个 variant 的 required 字段必须 ⊇ `AgentAction` 该 variant 的非空字段；
/// 6. 完整性闭环：把每个 variant 的最小合法实例 round-trip
///    `serde_json::from_value::<AgentAction>` 必须成功。
#[test]
fn plan_result_schema_drift() {
    const SCHEMA_RAW: &str = include_str!("../../../../prompts/schemas/plan_result.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("plan_result.schema.json must be valid JSON");
    assert_eq!(
        schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "schema root must be object"
    );
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&json!(false)),
        "schema root must reject unknown envelope fields after canonicalization"
    );
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema must have `required`");
    assert!(
        required.iter().any(|v| v.as_str() == Some("steps")),
        "envelope must require `steps`"
    );
    let defs = schema
        .get("$defs")
        .and_then(|v| v.as_object())
        .expect("schema must declare $defs");
    let action = defs
        .get("AgentAction")
        .expect("$defs.AgentAction must exist");
    let one_of = action
        .get("oneOf")
        .and_then(|v| v.as_array())
        .expect("AgentAction must be a oneOf union");

    // 期望与 `AgentAction` enum 完全对齐：think / call_skill / call_tool /
    // call_capability / synthesize_answer / respond
    let expected: HashSet<&str> = [
        "think",
        "call_skill",
        "call_tool",
        "call_capability",
        "synthesize_answer",
        "respond",
    ]
    .into_iter()
    .collect();
    let mut actual: HashSet<String> = HashSet::new();
    for entry in one_of {
        let ref_path = entry
            .get("$ref")
            .and_then(|v| v.as_str())
            .expect("oneOf entry must use $ref");
        let def_name = ref_path
            .strip_prefix("#/$defs/")
            .expect("$ref must point under #/$defs/");
        let def = defs.get(def_name).expect("referenced def must exist");
        assert_eq!(
            def.get("additionalProperties"),
            Some(&json!(false)),
            "variant `{}` must reject unknown action fields after canonicalization",
            def_name
        );
        let type_const = def
            .get("properties")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.get("const"))
            .and_then(|v| v.as_str())
            .expect("variant must declare `properties.type.const`");
        actual.insert(type_const.to_string());
    }
    let actual_refs: HashSet<&str> = actual.iter().map(String::as_str).collect();
    assert_eq!(
        actual_refs, expected,
        "plan_result.schema.json AgentAction oneOf must cover exactly {:?}, got {:?}",
        expected, actual_refs
    );

    // §D2.a 步骤 6：每个 variant 的最小合法实例必须能反序列化进 AgentAction。
    let probes: &[(&str, serde_json::Value)] = &[
        ("think", json!({"type": "think", "content": "x"})),
        (
            "call_skill",
            json!({"type": "call_skill", "skill": "run_cmd", "args": {}}),
        ),
        (
            "call_tool",
            json!({"type": "call_tool", "tool": "read_file", "args": {}}),
        ),
        (
            "call_capability",
            json!({"type": "call_capability", "capability": "filesystem.list_entries", "args": {}}),
        ),
        (
            "synthesize_answer",
            json!({"type": "synthesize_answer", "evidence_refs": ["last_output"]}),
        ),
        ("respond", json!({"type": "respond", "content": "ok"})),
    ];
    for (label, value) in probes {
        serde_json::from_value::<AgentAction>(value.clone()).unwrap_or_else(|err| {
                panic!(
                    "AgentAction variant `{}` failed to deserialize from schema-conformant minimum payload: {}",
                    label, err
                )
            });
    }
}
