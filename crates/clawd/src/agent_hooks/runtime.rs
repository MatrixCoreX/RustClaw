use serde_json::Value;

use super::shared::{
    handler_observation, pre_tool_hook_event, safe_handler_id, ExecutedHook, HandlerRunResult,
    HookFailurePolicy, HookHandlerConfig, LoadedHookConfiguration,
};
use super::{
    command, evaluate_pre_tool_use, http, mcp, merge_hook_decision, normalize_machine_token,
    HookEvaluation, HookPolicy, HookStage,
};
use crate::{policy_decision::PolicyDecision, AppState};

pub(crate) async fn pre_tool_use_outcome_for_state(
    state: &AppState,
    task_id: &str,
    tool_or_skill: &str,
    args: &Value,
) -> HookEvaluation {
    let action_ref = super::tool_action_ref(tool_or_skill, args);
    let loaded = load_hook_configuration(state);
    let mut outcome = evaluate_pre_tool_use(&loaded.policy, &action_ref);
    let mut handler_observations = Vec::new();
    if let Some(error_code) = loaded.error_code {
        merge_hook_decision(
            &mut outcome,
            PolicyDecision::Deny,
            "hook_config_invalid".to_string(),
        );
        handler_observations.push(handler_observation(
            "hook_config",
            "configuration",
            HookStage::PreToolUse,
            &action_ref,
            &HandlerRunResult::validation_failure(error_code),
            true,
            HookFailurePolicy::Deny.as_token(),
            "invalid",
            None,
        ));
        return HookEvaluation {
            outcome,
            handler_observations,
        };
    }
    if outcome.decision_kind() == Some(PolicyDecision::Deny) {
        return HookEvaluation {
            outcome,
            handler_observations,
        };
    }
    let event = pre_tool_hook_event(task_id, tool_or_skill, args, &action_ref);
    let cancellation = state
        .worker
        .task_cancellation_token(task_id)
        .unwrap_or_default();
    for handler in loaded
        .handlers
        .into_iter()
        .filter(|handler| handler.enabled)
    {
        let stage = match HookStage::parse_token(&handler.stage) {
            Some(stage) => stage,
            None => {
                record_validation_failure(
                    &mut outcome,
                    &mut handler_observations,
                    &handler,
                    HookStage::PreToolUse,
                    &action_ref,
                    "hook_handler_stage_invalid",
                );
                continue;
            }
        };
        if stage != HookStage::PreToolUse {
            continue;
        }
        let invalid_handler = handler.clone();
        let handler_kind = handler.kind.trim().to_string();
        let executed = match handler_kind.as_str() {
            "command" => {
                command::run_command_handler(
                    &state.skill_rt.workspace_root,
                    handler,
                    &event,
                    cancellation.clone(),
                )
                .await
            }
            "http" => http::run_http_handler(handler, &event, cancellation.clone()).await,
            "mcp" => {
                mcp::run_mcp_handler(
                    &state.core.mcp_runtime,
                    handler,
                    &event,
                    cancellation.clone(),
                )
                .await
            }
            _ => Err((
                safe_handler_id(&handler.id),
                "hook_handler_kind_unsupported",
            )),
        };
        match executed {
            Ok(executed) => record_execution(
                &mut outcome,
                &mut handler_observations,
                &action_ref,
                executed,
            ),
            Err((handler_id, error_code)) => {
                let mut invalid_handler = invalid_handler;
                invalid_handler.id = handler_id;
                record_validation_failure(
                    &mut outcome,
                    &mut handler_observations,
                    &invalid_handler,
                    stage,
                    &action_ref,
                    error_code,
                );
            }
        }
    }
    HookEvaluation {
        outcome,
        handler_observations,
    }
}

fn record_execution(
    outcome: &mut super::HookOutcome,
    observations: &mut Vec<Value>,
    action_ref: &str,
    executed: ExecutedHook,
) {
    merge_hook_decision(
        outcome,
        executed.result.decision,
        executed.result.reason_code.clone(),
    );
    observations.push(handler_observation(
        &executed.handler.id,
        executed.handler_kind,
        executed.handler.stage,
        action_ref,
        &executed.result,
        executed.handler.blocking,
        executed.handler.failure_policy.as_token(),
        executed.trust_status,
        executed.content_sha256.as_deref(),
    ));
}

fn record_validation_failure(
    outcome: &mut super::HookOutcome,
    observations: &mut Vec<Value>,
    handler: &HookHandlerConfig,
    stage: HookStage,
    action_ref: &str,
    error_code: &'static str,
) {
    let result = HandlerRunResult::validation_failure(error_code);
    merge_hook_decision(outcome, result.decision, result.reason_code.clone());
    observations.push(handler_observation(
        &safe_handler_id(&handler.id),
        &normalize_machine_token(&handler.kind),
        stage,
        action_ref,
        &result,
        handler.blocking,
        HookFailurePolicy::Deny.as_token(),
        "invalid",
        None,
    ));
}

fn load_hook_configuration(state: &AppState) -> LoadedHookConfiguration {
    let path = state
        .skill_rt
        .workspace_root
        .join("configs/agent_guard.toml");
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadedHookConfiguration {
                policy: HookPolicy::default(),
                handlers: Vec::new(),
                error_code: None,
            };
        }
        Err(_) => {
            return LoadedHookConfiguration {
                policy: HookPolicy::default(),
                handlers: Vec::new(),
                error_code: Some("hook_config_read_failed"),
            };
        }
    };
    let root = match toml::from_str::<toml::Value>(&raw) {
        Ok(root) => root,
        Err(_) => {
            return LoadedHookConfiguration {
                policy: HookPolicy::default(),
                handlers: Vec::new(),
                error_code: Some("hook_config_parse_failed"),
            };
        }
    };
    let policy = HookPolicy {
        blocked_action_refs: toml_string_array(&root, &["agent", "hooks", "blocked_action_refs"]),
        blocked_tools: toml_string_array(&root, &["agent", "hooks", "blocked_tools"]),
        require_confirmation_action_refs: toml_string_array(
            &root,
            &["agent", "hooks", "require_confirmation_action_refs"],
        ),
        background_wait_action_refs: toml_string_array(
            &root,
            &["agent", "hooks", "background_wait_action_refs"],
        ),
    };
    let handler_values = root
        .get("agent")
        .and_then(|value| value.get("hooks"))
        .and_then(|value| value.get("handlers"))
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut handlers = Vec::with_capacity(handler_values.len());
    let mut ids = std::collections::BTreeSet::new();
    for value in handler_values {
        let handler = match value.try_into::<HookHandlerConfig>() {
            Ok(handler) => handler,
            Err(_) => {
                return LoadedHookConfiguration {
                    policy,
                    handlers: Vec::new(),
                    error_code: Some("hook_handler_config_invalid"),
                };
            }
        };
        if handler.enabled && !ids.insert(handler.id.trim().to_string()) {
            return LoadedHookConfiguration {
                policy,
                handlers: Vec::new(),
                error_code: Some("hook_handler_id_duplicate"),
            };
        }
        handlers.push(handler);
    }
    LoadedHookConfiguration {
        policy,
        handlers,
        error_code: None,
    }
}

fn toml_string_array(root: &toml::Value, path: &[&str]) -> Vec<String> {
    let mut cursor = root;
    for segment in path {
        let Some(next) = cursor.get(*segment) else {
            return Vec::new();
        };
        cursor = next;
    }
    cursor
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(normalize_machine_token)
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
