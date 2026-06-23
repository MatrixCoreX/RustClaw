use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;

use serde_json::json;

use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
use crate::{
    contract_matrix::FailureAttribution, AgentRuntimeConfig, AppState, ClaimedTask, PlanKind,
    PlanResult, PlanStep, RouteResult, ScheduleKind, SkillViewsSnapshot, ToolsPolicy,
};

fn test_registry() -> SkillsRegistry {
    let toml = r#"
[[skills]]
name = "read_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["command"], properties = { command = { type = "string" } } }

[[skills]]
name = "list_dir"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path", "content"], properties = { path = { type = "string" }, content = { type = "string" } } }

[[skills]]
name = "make_dir"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "remove_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "fs_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["action"], properties = { action = { type = "string" }, path = { type = "string" }, paths = { type = "array", items = { type = "string" } } } }
planner_capabilities = [
  { name = "filesystem.stat_paths", action = "stat_paths", effect = "observe", required = ["path|paths"] },
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe", required = ["path"] },
  { name = "filesystem.remove_path", action = "remove_path", effect = "mutate", required = ["path"], risk_level = "high" },
]

[[skills]]
name = "system_basic"
enabled = true
kind = "runner"
planner_kind = "tool"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, kind = { type = "string" } } }
planner_capabilities = [
  { name = "system.runtime_status", action = "runtime_status", effect = "observe", optional = ["kind"], risk_level = "low", preferred = true },
]

[[skills]]
name = "package_manager"
enabled = true
kind = "builtin"
planner_kind = "skill"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, package = { type = "string" }, dry_run = { type = "boolean" } } }
planner_capabilities = [
  { name = "package.detect_manager", action = "detect", effect = "observe" },
  { name = "package.install", action = "install", effect = "mutate", required = ["package"], risk_level = "high" },
]

[[skills]]
name = "db_basic"
enabled = true
kind = "builtin"
planner_kind = "skill"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, db_path = { type = "string" }, sql = { type = "string" }, confirm = { type = "boolean" } } }
planner_capabilities = [
  { name = "database.query", action = "sqlite_query", effect = "observe", required = ["sql"] },
  { name = "database.schema_version", action = "schema_version", effect = "observe" },
  { name = "database.execute", action = "sqlite_execute", effect = "mutate", required = ["sql", "confirm"], risk_level = "high" },
]

[[skills]]
name = "config_edit"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "text"
side_effect = true
requires_confirmation = true
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, path = { type = "string" }, field_path = { type = "string" }, value = { type = "string" } } }
planner_capabilities = [
  { name = "config.plan_change", action = "plan_config_change", effect = "observe", required = ["field_path", "value"], risk_level = "low" },
  { name = "config.apply_change", action = "apply_config_change", effect = "mutate", required = ["field_path", "value"], risk_level = "high" },
]

[[skills]]
name = "audio_synthesize"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["text"], properties = { text = { type = "string" }, output_path = { type = "string" } } }
planner_capabilities = [
  { name = "audio.synthesize", action = "synthesize", effect = "external", required = ["text"], optional = ["output_path"], risk_level = "high" },
]

[[skills]]
name = "image_generate"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "image"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["prompt"], properties = { prompt = { type = "string" }, output_path = { type = "string" } } }
planner_capabilities = [
  { name = "image.generate", action = "generate", effect = "external", required = ["prompt"], optional = ["output_path"], risk_level = "high" },
]

[[skills]]
name = "image_edit"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "image"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["instruction"], properties = { instruction = { type = "string" }, image = { type = "string" }, output_path = { type = "string" } } }
planner_capabilities = [
  { name = "image.edit", action = "edit", effect = "external", required = ["instruction"], optional = ["image", "output_path"], risk_level = "high" },
]

[[skills]]
name = "primary_reader"
enabled = true
kind = "runner"
output_kind = "text"
group = "reader"
primary_fallback_role = "primary"

	[[skills]]
	name = "fallback_reader"
	enabled = true
	kind = "runner"
	output_kind = "text"
	group = "reader"
	primary_fallback_role = "fallback"

	[[skills]]
	name = "photo_organize"
	enabled = true
	kind = "runner"
	output_kind = "text"
	risk_level = "high"
	auto_invocable = false
	requires_confirmation = true
	side_effect = true
	confirmation_exempt_when = [
	  { action = "prepare" },
	  { action = "organize", mode = "plan" },
	]
	"#;
    let path = std::env::temp_dir().join(format!(
        "verifier_registry_{}_{}_{}.toml",
        std::process::id(),
        crate::now_ts_u64(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, toml).expect("write registry");
    let registry = SkillsRegistry::load_from_path(&path).expect("load registry");
    let _ = std::fs::remove_file(path);
    registry
}

fn test_state() -> AppState {
    let registry = Arc::new(test_registry());
    let skills_list = Arc::new(
        [
            "read_file",
            "run_cmd",
            "list_dir",
            "write_file",
            "make_dir",
            "fs_basic",
            "system_basic",
            "package_manager",
            "db_basic",
            "audio_synthesize",
            "image_generate",
            "image_edit",
            "primary_reader",
            "fallback_reader",
            "photo_organize",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>(),
    );
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: Some(registry),
                skills_list,
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            locator_scan_max_depth: 3,
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

fn test_task() -> ClaimedTask {
    ClaimedTask {
        task_id: "task-verify".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn route_result(needs_clarify: bool) -> RouteResult {
    route_result_with_risk(needs_clarify, crate::RiskCeiling::Unknown)
}

fn route_result_with_semantic(semantic_kind: crate::OutputSemanticKind) -> RouteResult {
    let mut route = route_result(false);
    route.output_contract = crate::IntentOutputContract {
        semantic_kind,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    route
}

fn route_result_with_risk(needs_clarify: bool, risk_ceiling: crate::RiskCeiling) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "test".to_string(),
        needs_clarify,
        route_reason: "test".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: vec!["read_file".to_string()],
        risk_ceiling,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

fn plan_result(steps: Vec<PlanStep>) -> PlanResult {
    PlanResult {
        goal: "test".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps,
        planner_notes: String::new(),
        plan_kind: PlanKind::Single,
        raw_plan_text: String::new(),
    }
}

#[test]
fn observe_mode_keeps_route_clarify_as_shadow_only() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(true)),
            request_text: None,
            context_bundle_summary: Some("need more info"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "read_file".to_string(),
                args: json!({ "path": "README.md" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.approved);
    assert!(result.blocked_reason.is_none());
    assert!(matches!(
        result.issues.first().map(|issue| issue.kind),
        Some(VerifyIssueKind::RouteClarifyRequired)
    ));
    assert!(result.shadow_blocked_reason.is_some());
}

#[test]
fn locatorless_runtime_status_plan_does_not_trip_route_clarify_block() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(true);
    route.route_reason = "locatorless_observation_requires_clarify".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: Some("runtime status scalar"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "system_basic".to_string(),
                args: json!({ "action": "runtime_status", "kind": "current_user" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result
        .issues
        .iter()
        .all(|issue| !matches!(issue.kind, VerifyIssueKind::RouteClarifyRequired)));
}

#[test]
fn observe_mode_rewrites_unresolved_template_args_to_response() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(true)),
            request_text: Some("帮我转成表格"),
            context_bundle_summary: Some("needs concrete JSON array"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "read_file".to_string(),
                args: json!({ "path": "{{last_output}}" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.approved);
    assert!(result.shadow_blocked_reason.is_some());
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::RouteClarifyRequired)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg)));
    assert_eq!(result.rewritten_steps.len(), 1);
    assert_eq!(result.rewritten_steps[0].action_type, "respond");
    let content = result.rewritten_steps[0]
        .args
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(content).expect("machine payload");
    assert_eq!(
        payload
            .get("reason_code")
            .and_then(serde_json::Value::as_str),
        Some("verify_unresolved_template_arg")
    );
    assert_eq!(
        payload
            .get("message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.verify.unresolved_template_arg")
    );
}

#[test]
fn observe_mode_rewrites_unresolved_call_capability_to_response() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("帮我查一下"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_capability".to_string(),
                skill: "unknown.example".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result.shadow_blocked_reason.is_some());
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::CapabilityUnavailable)));
    assert_eq!(result.rewritten_steps.len(), 1);
    assert_eq!(result.rewritten_steps[0].action_type, "respond");
    let content = result.rewritten_steps[0]
        .args
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(content).expect("machine payload");
    assert_eq!(
        payload
            .get("reason_code")
            .and_then(serde_json::Value::as_str),
        Some("verify_capability_unavailable")
    );
    assert_eq!(
        payload
            .get("capability")
            .and_then(serde_json::Value::as_str),
        Some("unknown.example")
    );
}

#[test]
fn enforce_mode_blocks_unresolved_call_capability() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_capability".to_string(),
                skill: "unknown.example".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(result.blocked_reason.is_some());
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::CapabilityUnavailable)));
}

#[test]
fn observe_mode_allows_prior_output_template_in_later_args() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(true)),
            request_text: Some(
                "查看 logs 目录，把里面的日志文件名整理到 logs_inventory.txt，然后把文件发给我。",
            ),
            context_bundle_summary: Some("auto_locator_path=/home/guagua/rustclaw/logs"),
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "list_dir".to_string(),
                    args: json!({ "path": "/home/guagua/rustclaw/logs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({
                        "path": "/home/guagua/rustclaw/logs_inventory.txt",
                        "content": "{{last_output}}"
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg)));
    assert!(result.rewritten_steps.is_empty());
}

#[test]
fn enforce_mode_blocks_missing_required_arg() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "read_file".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(matches!(
        result.issues.first().map(|issue| issue.kind),
        Some(VerifyIssueKind::MissingRequiredArg)
    ));
    assert!(result
        .blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("missing required arg"));
}

#[test]
fn enforce_mode_blocks_action_scoped_required_arg() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "read_text_range"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| matches!(
        issue.kind,
        VerifyIssueKind::MissingRequiredArg
    ) && issue.detail.contains("`path`")));
}

#[test]
fn enforce_mode_accepts_action_scoped_alternative_arg() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "stat_paths", "path": "README.md"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
}

#[test]
fn enforce_mode_blocks_mutation_above_low_risk_ceiling() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result_with_risk(false, crate::RiskCeiling::Low)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "write_file".to_string(),
                args: json!({"path": "out.txt", "content": "hello"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::RiskBudgetExceeded)));
    assert_eq!(
        result
            .permission_decision
            .pointer("/owner_layer")
            .and_then(serde_json::Value::as_str),
        Some("plan_verifier")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/status_code")
            .and_then(serde_json::Value::as_str),
        Some("risk_budget_exceeded")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/risk_level")
            .and_then(serde_json::Value::as_str),
        Some("high")
    );
}

#[test]
fn observe_mode_records_contract_action_rejection_for_structured_route() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_semantic(crate::OutputSemanticKind::FileNames);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({"command": "ls"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(issue.kind, VerifyIssueKind::ContractActionRejected)
            && issue.kind.failure_attribution() == FailureAttribution::ContractGap
    }));
    assert!(result
        .shadow_blocked_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("rejected by contract")));
}

#[test]
fn observe_mode_allows_user_named_output_path_marker_without_contract_rejection() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_semantic(crate::OutputSemanticKind::RawCommandOutput);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "write_file".to_string(),
                args: json!({
                    "path": "pwd_line_abs.txt",
                    "content": "/home/guagua/rustclaw\n",
                    "_clawd_user_named_output_path": true
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
}

#[test]
fn summary_contract_allows_registry_observe_config_preview_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_semantic(crate::OutputSemanticKind::CommandOutputSummary);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "config_edit".to_string(),
                args: json!({
                    "action": "plan_config_change",
                    "path": "configs/config.toml",
                    "field_path": "llm.selected_vendor",
                    "value": "minimax"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn summary_contract_still_rejects_registry_mutating_config_apply() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_semantic(crate::OutputSemanticKind::CommandOutputSummary);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "config_edit".to_string(),
                args: json!({
                    "action": "apply_config_change",
                    "path": "configs/config.toml",
                    "field_path": "llm.selected_vendor",
                    "value": "minimax"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn verifier_issue_failure_attribution_groups_contract_policy_kinds() {
    assert_eq!(
        VerifyIssueKind::ContractActionRejected.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::ContractMissing.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::ContractPolicyViolation.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::ContractPreferredActionAvailable.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::MissingRequiredArg.failure_attribution(),
        FailureAttribution::ModelError
    );
    assert_eq!(
        VerifyIssueKind::CapabilityUnavailable.failure_attribution(),
        FailureAttribution::ToolGap
    );
    assert_eq!(
        VerifyIssueKind::RiskBudgetExceeded.failure_attribution(),
        FailureAttribution::PermissionDenied
    );
}

#[test]
fn verifier_issue_kinds_expose_stable_machine_fields() {
    let kinds = [
        VerifyIssueKind::SkillNotVisible,
        VerifyIssueKind::CapabilityUnavailable,
        VerifyIssueKind::MissingRequiredArg,
        VerifyIssueKind::DefaultCreationTargetApplied,
        VerifyIssueKind::UnresolvedTemplateArg,
        VerifyIssueKind::InvalidDependsOn,
        VerifyIssueKind::ConfirmationRequired,
        VerifyIssueKind::RiskBudgetExceeded,
        VerifyIssueKind::PrimaryFallbackConflict,
        VerifyIssueKind::RouteClarifyRequired,
        VerifyIssueKind::RecipeInspectBeforeMutateRequired,
        VerifyIssueKind::RecipeValidationAfterMutateRequired,
        VerifyIssueKind::RecipeTargetScopeRequired,
        VerifyIssueKind::ContractActionRejected,
        VerifyIssueKind::ContractMissing,
        VerifyIssueKind::ContractPolicyViolation,
        VerifyIssueKind::ContractPreferredActionAvailable,
    ];

    for kind in kinds {
        assert!(!kind.reason_code().is_empty(), "{kind:?} reason_code");
        assert!(
            kind.reason_code().starts_with("verify_"),
            "{kind:?} reason_code prefix"
        );
        assert!(!kind.status_code().is_empty(), "{kind:?} status_code");
        assert!(
            kind.message_key().starts_with("clawd.verify."),
            "{kind:?} message_key namespace"
        );
        assert!(
            !kind.failure_attribution().as_str().is_empty(),
            "{kind:?} failure attribution"
        );
    }
}

#[test]
fn observe_mode_records_preferred_contract_action_without_blocking() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_semantic(crate::OutputSemanticKind::FileNames);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "find_entries", "path": "."}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result.issues.iter().any(|issue| matches!(
        issue.kind,
        VerifyIssueKind::ContractPreferredActionAvailable
    )));
    assert!(result.blocked_reason.is_none());
}

#[test]
fn generated_file_path_report_repairs_plan_with_missing_write_step() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_semantic(crate::OutputSemanticKind::GeneratedFilePathReport);
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_hint = "pwd_line_abs.txt".to_string();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({"command": "pwd"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "synthesize_answer".to_string(),
                    skill: "synthesize_answer".to_string(),
                    args: json!({"evidence_refs": ["last_output"]}),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert_eq!(result.approved_steps.len(), 3);
    let write_step = &result.approved_steps[1];
    assert_eq!(write_step.action_type, "call_tool");
    assert_eq!(write_step.skill, "fs_basic");
    assert_eq!(
        write_step
            .args
            .get("action")
            .and_then(|value| value.as_str()),
        Some("write_text")
    );
    assert_eq!(
        write_step
            .args
            .get("content")
            .and_then(|value| value.as_str()),
        Some("{{last_output}}")
    );
    assert!(
        write_step.args.get("text").is_none(),
        "generated_file_path_report repair must use the canonical fs_basic.write_text content arg"
    );
    let path = write_step
        .args
        .get("path")
        .and_then(|value| value.as_str())
        .expect("repaired write path");
    assert!(path.ends_with("pwd_line_abs.txt"), "path={path}");
    assert!(std::path::Path::new(path).is_absolute(), "path={path}");
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg)));
}

#[test]
fn enforce_mode_blocks_skill_switch_disabled_even_when_contract_allows_action() {
    let mut state = test_state();
    let registry = state
        .get_skills_registry()
        .expect("test registry should be installed");
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(
            ["read_file", "run_cmd", "list_dir"]
                .into_iter()
                .map(str::to_string)
                .collect::<HashSet<_>>(),
        ),
    })));
    let task = test_task();
    let route = route_result_with_semantic(crate::OutputSemanticKind::FileNames);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "list_dir", "path": "."}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved, "issues: {:?}", result.issues);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::SkillNotVisible)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(result
        .blocked_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("not in planner visible skills")));
}

#[test]
fn enforce_mode_allows_low_risk_action_under_low_risk_ceiling() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result_with_risk(false, crate::RiskCeiling::Low)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "stat_paths", "paths": ["README.md"]}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::RiskBudgetExceeded)));
}

#[test]
fn enforce_mode_blocks_skill_not_visible() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "totally_fake_skill".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result
        .issues
        .iter()
        .any(|issue| { matches!(issue.kind, VerifyIssueKind::SkillNotVisible) }));
}

#[test]
fn enforce_mode_blocks_primary_fallback_conflict() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "primary_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "fallback_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result
        .issues
        .iter()
        .any(|issue| { matches!(issue.kind, VerifyIssueKind::PrimaryFallbackConflict) }));
}

#[test]
fn verifier_allows_repeated_steps_from_same_primary_group_skill() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "primary_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "primary_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result
        .issues
        .iter()
        .all(|issue| { !matches!(issue.kind, VerifyIssueKind::PrimaryFallbackConflict) }));
}

#[test]
fn resume_execute_route_skips_confirmation_requirement() {
    let state = test_state();
    let task = test_task();
    let mut resumed_route = route_result(false);
    resumed_route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&resumed_route),
            request_text: None,
            context_bundle_summary: Some("resume"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({ "command": "pwd" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn confirmation_exempt_invocation_skips_confirmation_requirement() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: Some("photo preview"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "photo_organize".to_string(),
                args: json!({ "action": "organize", "mode": "plan" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn safe_make_dir_missing_path_defaults_under_workspace_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("帮我创建一个文件夹"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "make_dir".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
    let path = result.approved_steps[0]
        .args
        .get("path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    assert!(path.starts_with(state.skill_rt.workspace_root.to_string_lossy().as_ref()));
    assert!(path.contains("rustclaw-created-dir-taskveri"));
}

#[test]
fn safe_write_file_relative_path_anchors_under_workspace_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let filename = format!("rustclaw-autonomy-{}.txt", uuid::Uuid::new_v4());
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("把结果写到文件"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "write_file".to_string(),
                args: json!({ "path": filename, "content": "ok" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    let path = result.approved_steps[0]
        .args
        .get("path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    assert!(path.starts_with(state.skill_rt.workspace_root.to_string_lossy().as_ref()));
    assert!(path.ends_with(".txt"));
}

#[test]
fn dangerous_remove_file_missing_path_blocks_without_default_target() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("delete it"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "remove_file".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
}

#[test]
fn dangerous_fs_basic_remove_path_missing_path_blocks_without_default_target() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("remove that path"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({ "action": "remove_path" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
}

#[test]
fn destructive_run_cmd_requires_confirmation_without_resume() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("remove temp files"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({ "command": "rm -rf /tmp/rustclaw-verifier-test" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    assert_eq!(
        result
            .permission_decision
            .pointer("/allowed")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/decision")
            .and_then(serde_json::Value::as_str),
        Some("require_confirmation")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("verify_confirmation_required")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("require_confirmation")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/requires_confirmation")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn non_exempt_invocation_still_requires_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: Some("photo move"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "photo_organize".to_string(),
                args: json!({ "action": "organize", "mode": "move" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved);
    assert!(result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn ops_recipe_requires_inspect_before_mutate() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({ "command": "systemctl restart sing-box" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                    ..Default::default()
                },
            ),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}

#[test]
fn ops_recipe_requires_validation_after_mutate() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "configs/config.toml" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "systemctl restart sing-box" }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                    ..Default::default()
                },
            ),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            VerifyIssueKind::RecipeValidationAfterMutateRequired
        )
    }));
}

#[test]
fn code_change_recipe_requires_profile_specific_verification() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译或测试通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s3".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: vec!["s2".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    let issue = result
        .issues
        .iter()
        .find(|issue| {
            matches!(
                issue.kind,
                VerifyIssueKind::RecipeValidationAfterMutateRequired
            )
        })
        .expect("expected code_change validation issue");
    assert!(issue
        .detail
        .contains("code_change requires compile/test/build or runtime verification"));
}

#[test]
fn code_change_recipe_accepts_structured_cargo_check_verification() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s0".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: vec!["s0".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cargo check -p clawd",
                        "_clawd_validation": {
                            "profile": "code_change",
                            "validator_type": "build",
                            "validated_target": "clawd"
                        }
                    }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::RecipeValidationAfterMutateRequired
                | VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}

#[test]
fn code_change_recipe_accepts_run_cmd_cargo_check_verification() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s0".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: vec!["s0".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "cargo check -p clawd" }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result.issues.iter().all(|issue| !matches!(
        issue.kind,
        VerifyIssueKind::RecipeValidationAfterMutateRequired
    )));
}

#[test]
fn code_change_recipe_accepts_structured_custom_validation_step() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("修复当前仓库里的脚本，并运行自定义检查脚本验证通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s0".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "scripts/check.sh" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "scripts/check.sh", "content": "#!/usr/bin/env bash\nexit 0\n" }),
                    depends_on: vec!["s0".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "bash scripts/check.sh",
                        "_clawd_validation": {
                            "profile": "code_change",
                            "validator_type": "custom",
                            "validated_target": "scripts/check.sh"
                        }
                    }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::RecipeValidationAfterMutateRequired
                | VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}

#[path = "verifier_tests/scope_boundaries.rs"]
mod scope_boundaries;

#[path = "verifier_tests/scope_persistence.rs"]
mod scope_persistence;

#[path = "verifier_tests/ops_recipe_repair.rs"]
mod ops_recipe_repair;

#[path = "verifier_tests/media_artifact.rs"]
mod media_artifact;

#[path = "verifier_tests/registry_confirmation.rs"]
mod registry_confirmation;
