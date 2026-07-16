use serde_json::Value;
use tracing::{info, warn};

use super::super::planning_parse::parse_single_plan_actions;
use super::super::planning_prompt::{runtime_os_label, runtime_shell_label};
use super::super::PLANNER_ABORT_COMPACT_RETRY_PROMPT_LOGICAL_PATH;
use super::LoopState;
use crate::{llm_gateway, AgentAction, AppState, ClaimedTask, RouteResult};

const MAX_BLOCK_CHARS: usize = 7_000;
const MAX_INVALID_OUTPUT_CHARS: usize = 1_400;

pub(super) struct PlannerAbortRecoveryInput<'a> {
    pub(super) goal: &'a str,
    pub(super) turn_analysis: &'a str,
    pub(super) user_text: &'a str,
    pub(super) tool_spec: &'a str,
    pub(super) skill_playbooks: &'a str,
    pub(super) attempt_ledger: &'a str,
    pub(super) first_raw_plan: &'a str,
    pub(super) latest_raw_plan: Option<&'a str>,
    pub(super) round_no: usize,
    pub(super) route_result: Option<&'a RouteResult>,
    pub(super) loop_state: &'a LoopState,
}

pub(super) async fn compact_retry_plan_actions(
    state: &AppState,
    task: &ClaimedTask,
    input: PlannerAbortRecoveryInput<'_>,
) -> Result<Option<(Vec<AgentAction>, String)>, String> {
    if !should_try_compact_planner_abort_recovery(input.route_result, input.loop_state) {
        return Ok(None);
    }

    let raw = run_compact_retry_prompt(state, task, &input).await?;
    let Some(actions) = parse_single_plan_actions(&raw, state, task).await else {
        warn!(
            "planner_abort_compact_retry_parse_failed task_id={} round={}",
            task.task_id, input.round_no
        );
        return Ok(None);
    };
    Ok(Some((actions, raw)))
}

pub(super) fn should_try_compact_planner_abort_recovery(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> bool {
    if loop_state.execution_recipe.is_active() {
        return true;
    }
    route_result.is_some_and(|route| {
        !route.needs_clarify
            && (matches!(route.gate_kind(), crate::RouteGateKind::Execute)
                || route.has_route_reason_machine_marker(
                    "executable_contract_preserved_for_agent_loop",
                ))
    })
}

async fn run_compact_retry_prompt(
    state: &AppState,
    task: &ClaimedTask,
    input: &PlannerAbortRecoveryInput<'_>,
) -> Result<String, String> {
    let resolved_prompt = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        PLANNER_ABORT_COMPACT_RETRY_PROMPT_LOGICAL_PATH,
    )
    .map_err(|err| err.to_string())?;
    let prompt_source = resolved_prompt.source;
    let prompt_version = resolved_prompt.version;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, input.user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, input.user_text);
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let route_contract_summary = route_contract_summary(input.route_result);
    let invalid_plan_summary =
        invalid_plan_summary(input.first_raw_plan, input.latest_raw_plan.unwrap_or(""));
    let tool_spec = truncate_chars(input.tool_spec, MAX_BLOCK_CHARS);
    let skill_playbooks = truncate_chars(input.skill_playbooks, MAX_BLOCK_CHARS);
    let attempt_ledger = truncate_chars(input.attempt_ledger, MAX_BLOCK_CHARS);
    let prompt = crate::render_prompt_template(
        &resolved_prompt.template,
        &[
            ("__GOAL__", input.goal),
            ("__TURN_ANALYSIS__", input.turn_analysis),
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            ("__ROUTE_CONTRACT_SUMMARY__", &route_contract_summary),
            ("__TOOL_SPEC__", &tool_spec),
            ("__SKILL_PLAYBOOKS__", &skill_playbooks),
            ("__ATTEMPT_LEDGER__", &attempt_ledger),
            ("__INVALID_PLAN_SUMMARY__", &invalid_plan_summary),
            ("__RUNTIME_OS__", &runtime_os),
            ("__RUNTIME_SHELL__", &runtime_shell),
            ("__WORKSPACE_ROOT__", &workspace_root),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "planner_abort_compact_retry_prompt",
        &prompt_source,
        prompt_version.as_deref(),
        Some(input.round_no),
    );
    let raw =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await?;
    info!(
        "planner_abort_compact_retry_response task_id={} round={} raw={}",
        task.task_id,
        input.round_no,
        crate::truncate_for_log(&raw)
    );
    Ok(raw)
}

fn route_contract_summary(route_result: Option<&RouteResult>) -> String {
    let Some(route) = route_result else {
        return "{}".to_string();
    };
    let contract = &route.output_contract;
    let value = serde_json::json!({
        "ask_mode": route.route_trace_label_for_log(),
        "gate_kind": route.gate_kind().as_str(),
        "needs_clarify": route.needs_clarify,
        "route_reason": route.route_reason,
        "resolved_intent": route.resolved_intent,
        "risk_ceiling": route.risk_ceiling.as_str(),
        "resume_behavior": resume_behavior_label(route.resume_behavior),
        "wants_file_delivery": route.wants_file_delivery,
        "output_contract": {
            "response_shape": contract.response_shape.as_str(),
            "exact_sentence_count": contract.exact_sentence_count,
            "requires_content_evidence": contract.requires_content_evidence,
            "delivery_required": contract.delivery_required,
            "locator_kind": contract.locator_kind.as_str(),
            "delivery_intent": contract.delivery_intent.as_str(),
            "contract_marker": contract.semantic_kind.as_str(),
            "locator_hint": contract.locator_hint,
        }
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn resume_behavior_label(resume_behavior: crate::ResumeBehavior) -> &'static str {
    match resume_behavior {
        crate::ResumeBehavior::None => "none",
        crate::ResumeBehavior::ResumeExecute => "resume_execute",
        crate::ResumeBehavior::ResumeDiscuss => "resume_discuss",
    }
}

fn invalid_plan_summary(first_raw_plan: &str, latest_raw_plan: &str) -> String {
    let value = serde_json::json!({
        "first_raw_plan": invalid_output_entry(first_raw_plan),
        "latest_raw_plan": invalid_output_entry(latest_raw_plan),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn invalid_output_entry(raw: &str) -> Value {
    let trimmed = raw.trim();
    serde_json::json!({
        "empty": trimmed.is_empty(),
        "chars": raw.chars().count(),
        "preview": truncate_chars(trimmed, MAX_INVALID_OUTPUT_CHARS),
    })
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fixture_replay::{
        clear_cache_for_test, RecordedCall, FIXTURE_CALLS_FILENAME, FIXTURE_LLM_CASE_ENV,
        FIXTURE_LLM_ROOT_ENV, FIXTURE_LLM_SEQUENCE_FALLBACK_ENV,
    };
    use std::sync::Mutex;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    struct FixtureEnv {
        root: std::path::PathBuf,
    }

    impl FixtureEnv {
        fn install(clean_response: &str) -> Self {
            clear_cache_for_test();
            let root = std::env::temp_dir().join(format!(
                "rustclaw_planner_abort_recovery_{}",
                uuid::Uuid::new_v4()
            ));
            let case = "compact_retry";
            let case_dir = root.join(case);
            std::fs::create_dir_all(&case_dir).expect("create compact retry fixture dir");
            let rec = RecordedCall {
                prompt_hash: "0000000000000000".to_string(),
                prompt_source: Some("planner_abort_compact_retry".to_string()),
                prompt_preview: None,
                clean_response: clean_response.to_string(),
                raw_response: None,
                usage: None,
            };
            std::fs::write(
                case_dir.join(FIXTURE_CALLS_FILENAME),
                serde_json::to_string(&rec).expect("serialize fixture call"),
            )
            .expect("write compact retry fixture");
            std::env::set_var(FIXTURE_LLM_ROOT_ENV, &root);
            std::env::set_var(FIXTURE_LLM_CASE_ENV, case);
            std::env::set_var(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV, "1");
            Self { root }
        }
    }

    impl Drop for FixtureEnv {
        fn drop(&mut self) {
            std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
            std::env::remove_var(FIXTURE_LLM_CASE_ENV);
            std::env::remove_var(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV);
            clear_cache_for_test();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn route_with_mode(ask_mode: crate::AskMode, route_reason: &str) -> crate::RouteResult {
        crate::RouteResult {
            ask_mode,
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: route_reason.to_string(),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    #[test]
    fn compact_retry_is_limited_to_executable_routes_or_recipes() {
        let loop_state = LoopState::default();
        let chat_route = route_with_mode(crate::AskMode::state_patch_ack(), "");
        assert!(!should_try_compact_planner_abort_recovery(
            Some(&chat_route),
            &loop_state
        ));

        let execute_route = route_with_mode(crate::AskMode::act_plain(), "");
        assert!(should_try_compact_planner_abort_recovery(
            Some(&execute_route),
            &loop_state
        ));

        let marker_route = route_with_mode(
            crate::AskMode::state_patch_ack(),
            "executable_contract_preserved_for_agent_loop",
        );
        assert!(should_try_compact_planner_abort_recovery(
            Some(&marker_route),
            &loop_state
        ));
    }

    #[test]
    fn route_contract_summary_uses_machine_fields() {
        let mut route = route_with_mode(
            crate::AskMode::act_plain(),
            "executable_contract_preserved_for_agent_loop",
        );
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.locator_hint = "src/main.rs".to_string();

        let summary = route_contract_summary(Some(&route));
        assert!(summary.contains("\"gate_kind\": \"execute\""));
        assert!(summary.contains("\"response_shape\": \"scalar\""));
        assert!(summary.contains("\"locator_hint\": \"src/main.rs\""));
    }

    #[tokio::test]
    async fn compact_retry_uses_fixture_response_as_executable_steps() {
        let _lock = env_lock();
        let _fixture = FixtureEnv::install(
            r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"."}}]}"#,
        );
        let state =
            crate::AppState::test_default_with_fixture_provider().with_prompt_layers_installed();
        let task = crate::ClaimedTask {
            task_id: "compact-retry-task".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let route = route_with_mode(
            crate::AskMode::act_plain(),
            "executable_contract_preserved_for_agent_loop",
        );
        let loop_state = LoopState::new(1);

        let Some((actions, raw)) = compact_retry_plan_actions(
            &state,
            &task,
            PlannerAbortRecoveryInput {
                goal: "machine_goal",
                turn_analysis: "{}",
                user_text: "create project",
                tool_spec: "fs_basic",
                skill_playbooks: "",
                attempt_ledger: "[]",
                first_raw_plan: "",
                latest_raw_plan: Some(""),
                round_no: 1,
                route_result: Some(&route),
                loop_state: &loop_state,
            },
        )
        .await
        .expect("compact retry should run") else {
            panic!("compact retry should return actions");
        };

        assert!(raw.contains("\"steps\""));
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            AgentAction::CallTool { tool, args } => {
                assert_eq!(tool, "fs_basic");
                assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            }
            other => panic!("expected call_tool action, got {other:?}"),
        }
    }
}
