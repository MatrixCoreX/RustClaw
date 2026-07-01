use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::{
    executor::{StepExecutionResult, StepExecutionStatus},
    IntentOutputContract, OutputLocatorKind, OutputResponseShape, OutputScalarCountTargetKind,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, ScheduleKind,
};

fn route_with_contract(output_contract: IntentOutputContract) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "List the newest config files and judge their artifact kind.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract,
    }
}

fn planned_call<'a>(action: &'a AgentAction, skill: &str, action_name: &str) -> Option<&'a Value> {
    let (actual_skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    let actual_action = args.get("action").and_then(Value::as_str)?;
    (actual_skill == skill && actual_action == action_name).then_some(args)
}

fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
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

struct TempDirGuard {
    path: std::path::PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_recent_artifacts_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn recent_artifacts_judgment_keeps_config_file_content_reads() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = route_with_contract(IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: "configs".to_string(),
        ..IntentOutputContract::default()
    });
    let required = crate::task_contract::fallback_required_evidence_fields_for_output_contract(
        &route.output_contract,
    );
    assert!(required.contains(&"content_excerpt".to_string()));
    assert!(!required.contains(&"field_value".to_string()));

    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "configs/task_contract_matrix.toml",
                "mode": "head",
                "n": 15,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        None,
        actions,
    );

    assert!(
        normalized
            .iter()
            .any(|action| planned_call(action, "fs_basic", "read_text_range").is_some()),
        "normalized actions should keep bounded content reads: {normalized:?}"
    );
    assert!(
        normalized
            .iter()
            .all(|action| {
                planned_call(action, "config_basic", "read_field").is_none()
                    && planned_call(action, "config_basic", "read_fields").is_none()
            }),
        "recent artifact classification must not rewrite content reads to field reads: {normalized:?}"
    );
}

#[test]
fn recent_artifacts_judgment_expands_listing_only_to_selected_content_reads() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repo root")
        .to_path_buf();
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: "configs".to_string(),
        ..IntentOutputContract::default()
    };
    contract.self_extension.list_selector.target_kind = OutputScalarCountTargetKind::File;
    contract.self_extension.list_selector.target_kind_specified = true;
    contract.self_extension.list_selector.limit = Some(3);
    contract.self_extension.list_selector.sort_by = Some("mtime_desc".to_string());
    let route = route_with_contract(contract);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "configs",
            "sort_by": "mtime_desc",
            "max_entries": 3,
            "files_only": true,
            "names_only": false,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );

    assert!(
        planned_call(&normalized[0], "fs_basic", "list_dir").is_some(),
        "{normalized:?}"
    );
    let read_count = normalized
        .iter()
        .filter(|action| planned_call(action, "fs_basic", "read_text_range").is_some())
        .count();
    assert_eq!(read_count, 3, "{normalized:?}");
    assert!(
        normalized
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        "{normalized:?}"
    );
    assert!(
        matches!(normalized.last(), Some(AgentAction::Respond { content }) if content == "{{last_output}}"),
        "{normalized:?}"
    );
}

#[test]
fn recent_artifacts_judgment_any_selector_keeps_mixed_inventory() {
    let temp = TempDirGuard::new("mixed_inventory");
    std::fs::create_dir_all(temp.path.join("bundle_src")).expect("create bundle dir");
    std::fs::create_dir_all(temp.path.join("manual_unpack")).expect("create unpack dir");
    std::fs::write(temp.path.join("test_bundle.zip"), b"zip bytes").expect("write test bundle");

    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = temp.path.clone();
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: ".".to_string(),
        ..IntentOutputContract::default()
    };
    contract.self_extension.list_selector.limit = Some(3);
    contract.self_extension.list_selector.sort_by = Some("mtime_desc".to_string());
    let route = route_with_contract(contract);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": ".",
            "sort_by": "mtime_desc",
            "max_entries": 3,
            "files_only": true,
            "names_only": true,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );

    let listing_args =
        planned_call(&normalized[0], "fs_basic", "list_dir").expect("normalized listing");
    assert_eq!(
        listing_args.get("files_only").and_then(Value::as_bool),
        Some(false),
        "{normalized:?}"
    );
    assert_eq!(
        listing_args.get("dirs_only").and_then(Value::as_bool),
        Some(false),
        "{normalized:?}"
    );
    assert_eq!(
        listing_args.get("names_only").and_then(Value::as_bool),
        Some(false),
        "{normalized:?}"
    );
    assert_eq!(
        listing_args.get("max_entries").and_then(Value::as_u64),
        Some(3),
        "{normalized:?}"
    );
    assert_eq!(
        listing_args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc"),
        "{normalized:?}"
    );
    assert!(
        normalized
            .iter()
            .all(|action| planned_call(action, "fs_basic", "read_text_range").is_none()),
        "mixed selectors should not force binary file reads: {normalized:?}"
    );
}

#[test]
fn recent_artifacts_judgment_rewrites_synth_only_after_listing_to_selected_file_reads() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = route_with_contract(IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: "configs".to_string(),
        ..IntentOutputContract::default()
    });
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","entries":[{"kind":"file","path":"configs/task_contract_matrix.toml"},{"kind":"file","path":"configs/agent_guard.toml"},{"kind":"file","path":"configs/skills_registry.toml"}],"sort_by":"mtime_desc"},"text":"{}"}"#,
    ));
    let actions = vec![
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
        Some("configs"),
        actions,
    );

    let read_paths = normalized
        .iter()
        .filter_map(|action| planned_call(action, "fs_basic", "read_text_range"))
        .filter_map(|args| args.get("path").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        read_paths,
        vec![
            "configs/task_contract_matrix.toml",
            "configs/agent_guard.toml",
            "configs/skills_registry.toml"
        ],
        "{normalized:?}"
    );
    assert!(
        normalized
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        "{normalized:?}"
    );
}

#[test]
fn recent_artifacts_judgment_rewrites_repair_field_extract_to_selected_file_reads() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = route_with_contract(IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: "configs".to_string(),
        ..IntentOutputContract::default()
    });
    let mut loop_state = LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","entries":[{"kind":"file","path":"configs/task_contract_matrix.toml"},{"kind":"file","path":"configs/agent_guard.toml"},{"kind":"file","path":"configs/skills_registry.toml"}],"names":["task_contract_matrix.toml","agent_guard.toml","skills_registry.toml"],"sort_by":"mtime_desc"},"text":"{}"}"#,
    ));

    let actions = vec![
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "configs/task_contract_matrix.toml",
                "field_path": "runtime"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        Some("configs"),
        actions,
    );

    let read_paths = normalized
        .iter()
        .filter_map(|action| planned_call(action, "fs_basic", "read_text_range"))
        .filter_map(|args| args.get("path").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        read_paths,
        vec![
            "configs/task_contract_matrix.toml",
            "configs/agent_guard.toml",
            "configs/skills_registry.toml"
        ],
        "{normalized:?}"
    );
    assert!(
        normalized
            .iter()
            .all(|action| planned_call(action, "system_basic", "extract_field").is_none()),
        "field extraction should be rewritten away: {normalized:?}"
    );
    assert!(
        normalized
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        "rewritten plan should synthesize from bounded reads: {normalized:?}"
    );
}

#[test]
fn recent_artifacts_judgment_rewrites_capability_field_extract_to_selected_file_reads() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = route_with_contract(IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: "configs".to_string(),
        ..IntentOutputContract::default()
    });
    let mut loop_state = LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","entries":[{"kind":"file","path":"configs/task_contract_matrix.toml"},{"kind":"file","path":"configs/agent_guard.toml"},{"kind":"file","path":"configs/skills_registry.toml"}],"sort_by":"mtime_desc"},"text":"{}"}"#,
    ));

    let actions = vec![
        AgentAction::CallCapability {
            capability: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "configs/task_contract_matrix.toml",
                "field_path": "runtime"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        Some("configs"),
        actions,
    );

    let read_count = normalized
        .iter()
        .filter(|action| planned_call(action, "fs_basic", "read_text_range").is_some())
        .count();
    assert_eq!(read_count, 3, "{normalized:?}");
    assert!(
        normalized
            .iter()
            .all(|action| !matches!(action, AgentAction::CallCapability { .. })),
        "field extraction capability should be rewritten away: {normalized:?}"
    );
}

#[test]
fn recent_artifacts_judgment_uses_deterministic_listing_plan_before_open_planner() {
    let temp = TempDirGuard::new("deterministic_plan");
    std::fs::write(temp.path.join("clawd.run.log"), b"runtime log").expect("write log");
    std::fs::write(temp.path.join("nl_suite.log"), b"test log").expect("write log");
    let temp_path = temp.path.display().to_string();
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: temp_path.clone(),
        ..IntentOutputContract::default()
    };
    contract.self_extension.list_selector.limit = Some(2);
    contract.self_extension.list_selector.sort_by = Some("mtime_desc".to_string());
    contract.self_extension.list_selector.target_kind = OutputScalarCountTargetKind::File;
    contract.self_extension.list_selector.target_kind_specified = true;
    let route = route_with_contract(contract);

    let plan = recent_artifacts_judgment_deterministic_plan_result(
        "list recent artifacts and judge their kind",
        Some(&route),
        &LoopState::new(1),
        Some(temp_path.as_str()),
    )
    .expect("recent artifacts should use deterministic listing");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = planned_call(&action, "fs_basic", "list_dir").expect("list_dir action");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(temp_path.as_str())
    );
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(2));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc")
    );
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn recent_artifacts_contract_overrides_literal_command_guard_for_deterministic_plan() {
    let route = route_with_contract(IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        locator_hint: "logs".to_string(),
        ..IntentOutputContract::default()
    });

    assert!(structural_contract_deterministic_plan_overrides_literal_command_guard(Some(&route)));
}

#[test]
fn recent_artifacts_workspace_root_does_not_add_unsorted_tree_context_for_ranking() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = route_with_contract(IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        ..IntentOutputContract::default()
    });
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": ".",
                "sort_by": "mtime_desc",
                "max_entries": 3,
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
        &route.resolved_intent,
        Some("."),
        actions,
    );

    assert!(
        normalized
            .iter()
            .all(|action| planned_call(action, "system_basic", "tree_summary").is_none()),
        "root tree_summary is unsorted and must not replace mtime-ranked candidates: {normalized:?}"
    );
}
