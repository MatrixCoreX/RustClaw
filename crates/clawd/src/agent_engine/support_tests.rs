use super::{
    action_fingerprint, action_fingerprint_for_policy, append_delivery_message,
    collect_execution_recipe_progress_hints, execution_recipe_phase_progress_key,
    load_agent_loop_guard_policy, AgentLoopGuardPolicy, LoopBudgetProfile, LoopRecipeOverrides,
    SemanticRouteAuthority,
};
use crate::agent_engine::LoopState;
use crate::execution_recipe::{
    ExecutionRecipeKind, ExecutionRecipePhase, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
    ExecutionRecipeSpec, ExecutionRecipeTargetScope,
};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind, SkillViewsSnapshot,
};
use claw_core::skill_registry::SkillsRegistry;
use std::sync::{Arc, RwLock};

fn base_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_steps: 32,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 4,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        answer_verifier_enforce_required: false,
        semantic_route_authority: SemanticRouteAuthority::Legacy,
        agent_loop_canary_bucket: "none".to_string(),
        registry_idempotency_guard: false,
        structured_evidence_required_for_selected_contracts: false,
        fast_read: LoopRecipeOverrides {
            max_steps: Some(16),
            max_rounds: Some(2),
            max_tool_calls: Some(6),
            repeat_action_limit: Some(3),
            no_progress_limit: Some(1),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        grounded_summary: LoopRecipeOverrides {
            max_steps: Some(40),
            max_rounds: Some(4),
            max_tool_calls: Some(16),
            repeat_action_limit: Some(5),
            no_progress_limit: Some(2),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        multi_step_workspace: LoopRecipeOverrides {
            max_steps: Some(56),
            max_rounds: Some(6),
            max_tool_calls: Some(24),
            repeat_action_limit: Some(6),
            no_progress_limit: Some(2),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        ops_closed_loop: LoopRecipeOverrides {
            max_steps: Some(48),
            max_rounds: Some(4),
            max_tool_calls: Some(24),
            repeat_action_limit: Some(6),
            no_progress_limit: Some(2),
            max_repairs: Some(3),
            run_cmd_timeout_seconds: Some(180),
            run_cmd_validation_timeout_seconds: Some(90),
        },
    }
}

fn temp_support_workspace(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rustclaw-support-{name}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp support workspace");
    dir
}

fn state_with_registry(toml: &str, skills: &[&str]) -> crate::AppState {
    let root = temp_support_workspace("registry-policy");
    let path = root.join("skills_registry.toml");
    std::fs::write(&path, toml).expect("write registry");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry"));
    let _ = std::fs::remove_dir_all(root);
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(skills.iter().map(|skill| (*skill).to_string()).collect()),
    })));
    state
}

fn registry_governance_fixture() -> &'static str {
    r#"
[[skills]]
name = "config_edit"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "config.apply", action = "apply_config_change", effect = "mutate", once_per_task = true, dedup_scope = "action", idempotent = false },
]

[[skills]]
name = "fs_basic"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "filesystem.list_entries", action = "list_dir", effect = "observe", idempotent = true, dedup_scope = "args" },
]
"#
}

#[test]
fn rollout_switches_default_to_false_when_config_missing() {
    let root = temp_support_workspace("rollout-defaults");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert!(!policy.answer_verifier_enforce_required);
    assert_eq!(
        policy.semantic_route_authority,
        SemanticRouteAuthority::Legacy
    );
    assert!(!policy.records_agent_decides_attribution());
    assert_eq!(policy.agent_loop_canary_bucket, "none");
    assert!(!policy.registry_idempotency_guard);
    assert!(!policy.structured_evidence_required_for_selected_contracts);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn registry_idempotency_guard_switches_mutate_capability_to_action_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit", "fs_basic"]);
    let mut policy = base_policy();
    let left = crate::AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "apply_config_change",
            "field_path": "skills.a",
            "value": true
        }),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "apply_config_change",
            "field_path": "skills.b",
            "value": true
        }),
    };

    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );

    policy.registry_idempotency_guard = true;
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        "skill:config_edit:action:apply_config_change"
    );
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
}

#[test]
fn registry_idempotency_guard_keeps_observe_capability_args_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit", "fs_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard = true;
    let left = crate::AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({"action": "list_dir", "path": "/tmp/a"}),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({"action": "list_dir", "path": "/tmp/b"}),
    };

    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
}

#[test]
fn rollout_switches_are_read_from_agent_guard_config() {
    let root = temp_support_workspace("rollout-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
answer_verifier_enforce_required = true
agent_decides_semantic_route = true
agent_decides_migration_class = "structured_field_read"
registry_idempotency_guard = true
structured_evidence_required_for_selected_contracts = true
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert!(policy.answer_verifier_enforce_required);
    assert_eq!(
        policy.semantic_route_authority,
        SemanticRouteAuthority::Shadow
    );
    assert!(policy.records_agent_decides_attribution());
    assert_eq!(policy.agent_loop_canary_bucket, "structured_field_read");
    assert!(policy.registry_idempotency_guard);
    assert!(policy.structured_evidence_required_for_selected_contracts);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn semantic_route_authority_accepts_machine_tokens() {
    for (token, expected, records, agent_authority) in [
        ("legacy", SemanticRouteAuthority::Legacy, false, false),
        ("shadow", SemanticRouteAuthority::Shadow, true, false),
        (
            "agent_loop_canary",
            SemanticRouteAuthority::AgentLoopCanary,
            true,
            true,
        ),
        (
            "agent_loop_default",
            SemanticRouteAuthority::AgentLoopDefault,
            true,
            true,
        ),
    ] {
        let root = temp_support_workspace(&format!("semantic-authority-{token}"));
        let config_dir = root.join("configs");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::write(
            config_dir.join("agent_guard.toml"),
            format!(
                r#"
[agent.loop_guard]
semantic_route_authority = "{token}"
"#
            ),
        )
        .expect("write agent guard config");
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root.clone();

        let policy = load_agent_loop_guard_policy(&state);

        assert_eq!(policy.semantic_route_authority, expected);
        assert_eq!(policy.records_agent_decides_attribution(), records);
        assert_eq!(policy.uses_agent_loop_semantic_authority(), agent_authority);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn semantic_route_authority_rejects_freeform_text() {
    let root = temp_support_workspace("semantic-authority-invalid");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
semantic_route_authority = "let the agent decide from user text"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.semantic_route_authority,
        SemanticRouteAuthority::Legacy
    );
    assert!(!policy.records_agent_decides_attribution());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_agent_decides_migration_class_rejects_unknown_tokens() {
    let root = temp_support_workspace("agent-loop-canary-bucket-invalid");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
agent_decides_migration_class = "freeform_user_phrase"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(policy.agent_loop_canary_bucket, "none");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn agent_loop_canary_bucket_accepts_low_risk_tokens_only() {
    for token in [
        "none",
        "bound_path_summary",
        "structured_field_read",
        "exact_path_list",
        "recent_artifacts_judgment",
        "scalar_count",
        "low_risk_status_observation",
        "low_risk_config_read",
        "low_risk_log_observation",
        "low_risk_workspace_question",
        "low_risk_tool_discovery",
    ] {
        let root = temp_support_workspace(&format!("agent-decides-class-{token}"));
        let config_dir = root.join("configs");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::write(
            config_dir.join("agent_guard.toml"),
            format!(
                r#"
[agent.loop_guard]
agent_loop_canary_bucket = "{token}"
"#
            ),
        )
        .expect("write agent guard config");
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root.clone();

        let policy = load_agent_loop_guard_policy(&state);

        assert_eq!(policy.agent_loop_canary_bucket, token);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn deprecated_domain_action_lists_do_not_change_loop_guard_policy() {
    let root = temp_support_workspace("deprecated-domain-actions");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard.crypto]
news_actions = ["legacy_news"]
market_query_actions = ["legacy_quote"]
trade_preview_actions = ["legacy_preview"]
trade_submit_actions = ["legacy_submit"]

[agent.loop_guard.fs_search]
query_actions = ["legacy_find"]

[agent.loop_guard.media]
image_generate_skills = ["legacy_image_generate"]
image_edit_skills = ["legacy_image_edit"]
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(policy.max_rounds, 2);
    assert_eq!(policy.max_steps, 32);
    assert_eq!(policy.max_tool_calls, 12);
    assert!(!policy.registry_idempotency_guard);

    let _ = std::fs::remove_dir_all(root);
}

fn route_with_contract(
    semantic_kind: OutputSemanticKind,
    locator_kind: OutputLocatorKind,
) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn ops_closed_loop_policy_uses_override_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    let adjusted = policy.adjusted_for_context(recipe, None);
    assert_eq!(adjusted.max_steps, 48);
    assert_eq!(adjusted.max_rounds, 4);
    assert_eq!(adjusted.max_tool_calls, 24);
    assert_eq!(adjusted.repeat_action_limit, 6);
    assert_eq!(adjusted.no_progress_limit, 2);
    assert_eq!(
        adjusted.run_cmd_timeout_override(recipe, crate::execution_recipe::ActionEffect::mutate()),
        Some(180)
    );
    assert_eq!(
        adjusted
            .run_cmd_timeout_override(recipe, crate::execution_recipe::ActionEffect::validate()),
        Some(90)
    );
}

#[test]
fn route_contract_selects_grounded_summary_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::default();
    let route = route_with_contract(
        OutputSemanticKind::CommandOutputSummary,
        OutputLocatorKind::None,
    );

    assert_eq!(
        AgentLoopGuardPolicy::budget_profile_for_context(recipe, Some(&route)),
        LoopBudgetProfile::GroundedSummary
    );
    let adjusted = policy.adjusted_for_context(recipe, Some(&route));
    assert_eq!(adjusted.max_rounds, 4);
    assert_eq!(adjusted.max_tool_calls, 16);
    assert_eq!(adjusted.no_progress_limit, 2);
}

#[test]
fn workspace_delivery_contract_selects_multi_step_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::default();
    let mut route = route_with_contract(
        OutputSemanticKind::GeneratedFileDelivery,
        OutputLocatorKind::Filename,
    );
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::FileToken;

    assert_eq!(
        AgentLoopGuardPolicy::budget_profile_for_context(recipe, Some(&route)),
        LoopBudgetProfile::MultiStepWorkspace
    );
    let adjusted = policy.adjusted_for_context(recipe, Some(&route));
    assert_eq!(adjusted.max_rounds, 6);
    assert_eq!(adjusted.max_steps, 56);
    assert_eq!(adjusted.max_tool_calls, 24);
}

#[test]
fn ops_closed_loop_runtime_applies_repair_override() {
    let policy = base_policy();
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    policy.apply_recipe_runtime_overrides(&mut recipe);
    assert_eq!(recipe.max_repairs, 3);
}

#[test]
fn append_delivery_message_sanitizes_structured_skill_errors() {
    let mut messages = Vec::new();
    append_delivery_message(
        "task-support-test",
        &mut messages,
        r#"执行失败：__RC_SKILL_ERROR__:{"skill":"archive_basic","error_kind":"unknown","error_text":"archive is required","text":null}。"#
            .to_string(),
    );

    assert_eq!(messages, vec!["执行失败：archive is required。"]);
}

#[test]
fn external_workspace_progress_hints_include_mode_and_ready_once() {
    let mut loop_state = LoopState::new(4);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        target_scope: ExecutionRecipeTargetScope::ExternalWorkspace,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    });

    let first = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("telegram.progress.ops_recipe_scope_external_mode"));
    assert!(first[1].contains("telegram.progress.ops_recipe_inspect"));

    loop_state.execution_recipe.saw_external_target = true;
    let second = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(second.len(), 1);
    assert!(second[0].contains("telegram.progress.ops_recipe_scope_external_ready"));

    let third = collect_execution_recipe_progress_hints(&mut loop_state);
    assert!(third.is_empty());
}

#[test]
fn greenfield_progress_hints_include_mode_and_creation_ready_once() {
    let mut loop_state = LoopState::new(4);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        target_scope: ExecutionRecipeTargetScope::Greenfield,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    });

    let first = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("telegram.progress.ops_recipe_scope_greenfield_mode"));
    assert!(first[1].contains("telegram.progress.ops_recipe_inspect"));

    loop_state.execution_recipe.saw_greenfield_creation = true;
    let second = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(second.len(), 1);
    assert!(second[0].contains("telegram.progress.ops_recipe_scope_greenfield_ready"));

    let third = collect_execution_recipe_progress_hints(&mut loop_state);
    assert!(third.is_empty());
}

#[test]
fn code_change_phase_progress_uses_profile_specific_keys() {
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Inspect
        ),
        "telegram.progress.code_change_inspect"
    );
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Apply
        ),
        "telegram.progress.code_change_apply"
    );
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Validate
        ),
        "telegram.progress.code_change_validate"
    );
}

#[test]
fn skill_authoring_validate_progress_uses_profile_specific_key() {
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::SkillAuthoring,
            ExecutionRecipePhase::Validate
        ),
        "telegram.progress.skill_authoring_validate"
    );
}
