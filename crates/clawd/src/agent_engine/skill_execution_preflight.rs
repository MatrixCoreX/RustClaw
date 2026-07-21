use claw_core::skill_registry::SkillRiskLevel;
use serde_json::{json, Value};
use tracing::info;

use super::{register_failed_step_output, AppState, ClaimedTask, LoopState, SkillActionOutcome};
use crate::agent_engine::{
    attempt_ledger, maybe_publish_execution_recipe_phase_hint, CLAWD_LITERAL_COMMAND_ARG,
};

fn matches_json_schema_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "string" => value.is_string(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => true,
    }
}

fn validate_json_contract(value: &Value, schema: &Value) -> Result<(), String> {
    let expected_type = schema.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if !expected_type.is_empty() && !matches_json_schema_type(value, expected_type) {
        return Err(format!("expected type `{expected_type}`"));
    }
    if expected_type == "object" {
        let obj = value
            .as_object()
            .ok_or_else(|| "expected object output".to_string())?;
        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for key in required.iter().filter_map(|item| item.as_str()) {
                if !obj.contains_key(key) {
                    return Err(format!("missing required field `{key}`"));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
            for (key, prop_schema) in properties {
                let Some(field_value) = obj.get(key) else {
                    continue;
                };
                if let Some(field_type) = prop_schema.get("type").and_then(|v| v.as_str()) {
                    if !matches_json_schema_type(field_value, field_type) {
                        return Err(format!("field `{key}` expected type `{field_type}`"));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_skill_output_contract(
    state: &AppState,
    normalized_skill: &str,
    output: &str,
) -> Result<(), String> {
    let Some((output_kind, schema)) = state.skill_output_contract(normalized_skill) else {
        return Ok(());
    };
    let schema_accepts_text_object = schema.get("type").and_then(|v| v.as_str()) == Some("object")
        && schema
            .get("properties")
            .and_then(|v| v.as_object())
            .map(|props| props.contains_key("text"))
            .unwrap_or(false);
    let candidate = if schema_accepts_text_object {
        json!({ "text": output })
    } else if output_kind == claw_core::skill_registry::OutputKind::Text {
        Value::String(output.to_string())
    } else {
        crate::parse_llm_json_raw_or_any::<Value>(output)
            .unwrap_or_else(|| Value::String(output.to_string()))
    };
    validate_json_contract(&candidate, &schema)
}

fn string_contains_unresolved_runtime_placeholder(value: &str) -> bool {
    let mut search_start = 0usize;
    while let Some(open_rel) = value[search_start..].find("{{") {
        let inner_start = search_start + open_rel + 2;
        let Some(close_rel) = value[inner_start..].find("}}") else {
            return false;
        };
        let inner_end = inner_start + close_rel;
        if unresolved_runtime_placeholder_key(&value[inner_start..inner_end]) {
            return true;
        }
        search_start = inner_end + 2;
    }
    false
}

fn unresolved_runtime_placeholder_key(key: &str) -> bool {
    let key = key.trim();
    !key.is_empty()
        && key.len() <= 160
        && key.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '_' | '-' | '.' | '[' | ']' | '"' | '\'' | ' ' | '\t' | '\n'
                )
        })
}

pub(super) fn contains_unresolved_runtime_template_arg(value: &Value) -> bool {
    match value {
        Value::String(value) => string_contains_unresolved_runtime_placeholder(value),
        Value::Array(items) => items.iter().any(contains_unresolved_runtime_template_arg),
        Value::Object(map) => map.values().any(contains_unresolved_runtime_template_arg),
        _ => false,
    }
}

fn run_cmd_is_literal_user_command(args: &Value) -> bool {
    args.get(CLAWD_LITERAL_COMMAND_ARG)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn unresolved_runtime_template_argument_error(
    normalized_skill: &str,
    exec_args: &Value,
    classification_args: &Value,
) -> Option<String> {
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_is_literal_user_command(classification_args)
    {
        return None;
    }
    if !contains_unresolved_runtime_template_arg(exec_args) {
        return None;
    }
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "invalid_args",
        "execution argument still contains an unresolved runtime placeholder; replan with concrete observed values or use a single command pipeline",
        None,
        Some(json!({
            "reason": "unresolved_runtime_placeholder",
        })),
    ))
}

pub(super) fn evidence_policy_action_policy_error(
    state: &AppState,
    loop_state: &LoopState,
    normalized_skill: &str,
    classification_args: &Value,
    _action_trace_kind: &str,
) -> Option<String> {
    if matches!(
        normalized_skill,
        "synthesize_answer" | "respond" | "think" | "answer_verifier"
    ) {
        return None;
    }
    if let Some(err) = run_cmd_dry_run_policy_error(state, normalized_skill, classification_args) {
        return Some(err);
    }
    if let Some(err) =
        run_cmd_async_start_policy_error(state, normalized_skill, classification_args)
    {
        return Some(err);
    }
    if let Some(err) = generated_media_path_run_cmd_policy_error(
        state,
        loop_state,
        normalized_skill,
        classification_args,
    ) {
        return Some(err);
    }
    None
}

fn run_cmd_async_start_policy_error(
    state: &AppState,
    normalized_skill: &str,
    classification_args: &Value,
) -> Option<String> {
    if !normalized_skill.eq_ignore_ascii_case("run_cmd")
        || classification_args
            .get("async_start")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return None;
    }
    if positive_bounded_i64_arg(classification_args, "poll_after_seconds", 1, 86_400)
        && positive_bounded_i64_arg(classification_args, "expires_in_seconds", 1, 604_800)
    {
        return None;
    }
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "contract_action_rejected",
        "async_start_requires_bounded_poll_and_expiry",
        None,
        Some(json!({
            "reason_code": "async_start_requires_bounded_poll_and_expiry",
            "failure_attribution": crate::evidence_policy::FailureAttribution::ModelError.as_str(),
            "decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
            "action": "run_cmd",
            "required_fields": ["poll_after_seconds", "expires_in_seconds"],
            "permission_decision": preflight_permission_decision(
                state,
                normalized_skill,
                classification_args,
                "async_start_requires_bounded_poll_and_expiry",
                "run_cmd_async_start_preflight",
            ),
        })),
    ))
}

fn run_cmd_dry_run_policy_error(
    state: &AppState,
    normalized_skill: &str,
    classification_args: &Value,
) -> Option<String> {
    if !normalized_skill.eq_ignore_ascii_case("run_cmd")
        || !run_cmd_dry_run_requested(classification_args)
        || classification_args
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return None;
    }
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "contract_action_rejected",
        "run_cmd_dry_run_requires_preview_contract",
        None,
        Some(json!({
            "reason_code": "run_cmd_dry_run_requires_preview_contract",
            "message_key": "clawd.contract.run_cmd_dry_run_requires_preview_contract",
            "failure_attribution": crate::evidence_policy::FailureAttribution::ModelError.as_str(),
            "decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
            "action": "run_cmd",
            "dry_run": true,
            "required_contract": "pending_async_job_contract_preview",
            "forbidden_effect": "local_process_start",
            "permission_decision": preflight_permission_decision(
                state,
                normalized_skill,
                classification_args,
                "run_cmd_dry_run_requires_preview_contract",
                "run_cmd_dry_run_preflight",
            ),
        })),
    ))
}

fn run_cmd_dry_run_requested(classification_args: &Value) -> bool {
    classification_args.get("dry_run").and_then(Value::as_bool) == Some(true)
}

fn positive_bounded_i64_arg(args: &Value, key: &str, min: i64, max: i64) -> bool {
    args.get(key)
        .and_then(Value::as_i64)
        .is_some_and(|value| value >= min && value <= max)
}

fn generated_media_path_run_cmd_policy_error(
    state: &AppState,
    loop_state: &LoopState,
    normalized_skill: &str,
    classification_args: &Value,
) -> Option<String> {
    if !normalized_skill.eq_ignore_ascii_case("run_cmd")
        || run_cmd_is_literal_user_command(classification_args)
    {
        return None;
    }
    let output_contract = loop_state.output_contract.as_ref()?;
    if !output_contract.requests_exact_scalar_path()
        || !crate::media_artifact_paths::is_media_artifact_path(&output_contract.locator_hint)
    {
        return None;
    }
    let preferred_actions = [
        "image_edit",
        "image_generate",
        "audio_synthesize",
        "video_generate",
        "music_generate",
    ];
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "contract_action_rejected",
        "media_artifact_requires_media_skill",
        None,
        Some(json!({
            "reason_code": "media_artifact_requires_media_skill",
            "message_key": "clawd.contract.media_artifact_requires_media_skill",
            "failure_attribution": crate::evidence_policy::FailureAttribution::ModelError.as_str(),
            "decision": crate::evidence_policy::ActionPolicyDecision::RejectedNotAllowed.as_str(),
            "policy_decision": crate::policy_decision::PolicyDecision::from_evidence_action_policy(
                crate::evidence_policy::ActionPolicyDecision::RejectedNotAllowed
            ).as_token(),
            "action": "run_cmd",
            "original_action_ref": "run_cmd",
            "contract_match": "exact_scalar_path_selector",
            "preferred_actions": preferred_actions,
            "target_path": output_contract.locator_hint,
            "final_answer_shape": "single_path",
            "policy_mode": "enforce",
            "permission_decision": preflight_permission_decision(
                state,
                normalized_skill,
                classification_args,
                "media_artifact_requires_media_skill",
                "run_cmd_media_artifact_preflight",
            ),
        })),
    ))
}

fn risk_level_token(value: Option<SkillRiskLevel>) -> &'static str {
    match value.unwrap_or(SkillRiskLevel::Unknown) {
        SkillRiskLevel::Unknown => "unknown",
        SkillRiskLevel::Low => "low",
        SkillRiskLevel::Medium => "medium",
        SkillRiskLevel::High => "high",
    }
}

fn action_effect_token(effect: crate::execution_recipe::ActionEffect) -> &'static str {
    match (effect.observes, effect.mutates, effect.validates) {
        (_, true, true) => "mutate_validate",
        (_, true, false) => "mutate",
        (_, false, true) => "validate",
        (true, false, false) => "observe",
        _ => "unknown",
    }
}

fn run_cmd_command_policy(
    canonical_skill: &str,
    args: &Value,
    effect: crate::execution_recipe::ActionEffect,
) -> Option<Value> {
    if !canonical_skill.eq_ignore_ascii_case("run_cmd") {
        return None;
    }
    let literal_command_token = run_cmd_is_literal_user_command(args);
    Some(json!({
        "schema_version": 1,
        "policy_authority": if literal_command_token {
            "explicit_command_token"
        } else {
            "planner_structured_args"
        },
        "literal_command_token": literal_command_token,
        "command_arg_present": args
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        "unresolved_runtime_template_present": contains_unresolved_runtime_template_arg(args),
        "effect": action_effect_token(effect),
        "observes": effect.observes,
        "mutates": effect.mutates,
        "validates": effect.validates,
    }))
}

fn normalized_action_arg(args: &Value) -> Option<String> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .to_ascii_lowercase()
                .chars()
                .map(|ch| {
                    if matches!(ch, '-' | ' ' | '.') {
                        '_'
                    } else {
                        ch
                    }
                })
                .collect()
        })
}

fn action_scoped_risk_level(
    state: &AppState,
    canonical_skill: &str,
    action: Option<&str>,
) -> Option<SkillRiskLevel> {
    if let Some(tool) = state.mcp_tool(canonical_skill) {
        return Some(match tool.policy.risk_level.as_str() {
            "low" => SkillRiskLevel::Low,
            "medium" => SkillRiskLevel::Medium,
            "high" => SkillRiskLevel::High,
            _ => SkillRiskLevel::Unknown,
        });
    }
    state.skill_manifest(canonical_skill).and_then(|manifest| {
        claw_core::skill_registry::select_planner_capability_mapping(
            &manifest.planner_capabilities,
            action,
        )
        .and_then(|mapping| mapping.risk_level)
    })
}

fn action_scoped_capability_policy(
    state: &AppState,
    canonical_skill: &str,
    action: Option<&str>,
) -> Option<Value> {
    if let Some(tool) = state.mcp_tool(canonical_skill) {
        return Some(tool.policy.permission_policy_json());
    }
    state.skill_manifest(canonical_skill).and_then(|manifest| {
        claw_core::skill_registry::select_planner_capability_mapping(
            &manifest.planner_capabilities,
            action,
        )
        .map(|mapping| {
            json!({
                "isolation_profile": mapping
                    .isolation_profile
                    .map(|value| value.as_token()),
                "network_access": mapping.network_access,
                "filesystem_write": mapping.filesystem_write,
                "external_publish": mapping.external_publish,
                "credential_access": mapping.credential_access,
                "subprocess": mapping.subprocess,
                "package_install": mapping.package_install,
                "privilege_escalation": mapping.privilege_escalation,
            })
        })
    })
}

fn permission_path_arg_values(args: &Value) -> Vec<&str> {
    let Some(obj) = args.as_object() else {
        return Vec::new();
    };
    [
        "path",
        "root",
        "cwd",
        "output_path",
        "file_path",
        "target_path",
        "source_path",
        "destination_path",
    ]
    .into_iter()
    .filter_map(|key| {
        obj.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
    .collect()
}

fn permission_path_stays_in_workspace(workspace_root: &std::path::Path, raw_path: &str) -> bool {
    let candidate = std::path::Path::new(raw_path);
    if candidate.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        return false;
    }
    if !candidate.is_absolute() {
        return true;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let target = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    target.starts_with(root)
}

fn permission_workspace_scope_summary(
    state: &AppState,
    canonical_skill: &str,
    args: &Value,
) -> Value {
    let path_values = permission_path_arg_values(args);
    let untrusted_path_present = path_values
        .iter()
        .any(|path| !permission_path_stays_in_workspace(&state.skill_rt.workspace_root, path));
    let external_workspace =
        crate::execution_recipe::action_targets_external_workspace(state, canonical_skill, args);
    let scope = if untrusted_path_present || external_workspace {
        "external_or_untrusted"
    } else if path_values.is_empty() {
        "unspecified"
    } else {
        "workspace_scoped"
    };
    json!({
        "schema_version": 1,
        "scope": scope,
        "path_arg_count": path_values.len(),
        "cwd_present": args
            .get("cwd")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        "untrusted_path_present": untrusted_path_present,
        "external_workspace": external_workspace,
    })
}

fn sandbox_profile_token(
    capability_policy: &Value,
    effect: crate::execution_recipe::ActionEffect,
) -> String {
    capability_policy
        .get("isolation_profile")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if effect.mutates {
                "local_current_workspace".to_string()
            } else {
                "read_only_or_validation".to_string()
            }
        })
}

fn sandbox_policy_summary(
    capability_policy: &Value,
    sandbox_profile: &str,
    effect: crate::execution_recipe::ActionEffect,
) -> Value {
    json!({
        "schema_version": 1,
        "profile": sandbox_profile,
        "source": if capability_policy
            .get("isolation_profile")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            "registry_capability_policy"
        } else {
            "effect_default"
        },
        "filesystem_write": capability_policy
            .get("filesystem_write")
            .and_then(Value::as_bool)
            .unwrap_or(effect.mutates),
        "network_access": capability_policy.get("network_access").and_then(Value::as_bool),
        "external_publish": capability_policy
            .get("external_publish")
            .and_then(Value::as_bool),
        "credential_access": capability_policy
            .get("credential_access")
            .and_then(Value::as_bool),
        "subprocess": capability_policy
            .get("subprocess")
            .and_then(Value::as_bool),
        "package_install": capability_policy
            .get("package_install")
            .and_then(Value::as_bool),
        "privilege_escalation": capability_policy
            .get("privilege_escalation")
            .and_then(Value::as_bool),
    })
}

pub(super) fn capability_isolation_policy_error(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> Option<String> {
    let canonical_skill = state.resolve_canonical_skill_name(normalized_skill);
    let action = normalized_action_arg(args);
    let capability_policy =
        action_scoped_capability_policy(state, &canonical_skill, action.as_deref())?;
    let isolation_profile = capability_policy
        .get("isolation_profile")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let violations = isolation_profile_violations(isolation_profile, &capability_policy);
    if violations.is_empty() {
        return None;
    }
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "isolation_policy_violation",
        "isolation_policy_violation",
        None,
        Some(json!({
            "reason_code": "isolation_policy_violation",
            "failure_attribution": crate::evidence_policy::FailureAttribution::PermissionDenied.as_str(),
            "decision": crate::evidence_policy::ActionPolicyDecision::RejectedForbidden.as_str(),
            "policy_decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
            "canonical_skill": canonical_skill,
            "action": action,
            "isolation_profile": isolation_profile,
            "violations": violations,
            "capability_policy": capability_policy,
            "permission_decision": preflight_permission_decision(
                state,
                normalized_skill,
                args,
                "isolation_policy_violation",
                "capability_isolation_preflight",
            ),
        })),
    ))
}

pub(super) fn capability_isolation_artifact_refs(
    state: &AppState,
    task_id: &str,
    normalized_skill: &str,
    args: &Value,
) -> Vec<Value> {
    let canonical_skill = state.resolve_canonical_skill_name(normalized_skill);
    let action = normalized_action_arg(args);
    let Some(capability_policy) =
        action_scoped_capability_policy(state, &canonical_skill, action.as_deref())
    else {
        return Vec::new();
    };
    let Some(profile_token) = capability_policy
        .get("isolation_profile")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };
    let Some(profile) = crate::execution_isolation::isolation_profile_from_token(profile_token)
    else {
        return Vec::new();
    };
    let Ok(plan) = crate::execution_isolation::plan_execution_isolation(
        &state.skill_rt.workspace_root,
        task_id,
        profile,
    ) else {
        return Vec::new();
    };
    if !plan.requires_cleanup {
        return Vec::new();
    }
    vec![crate::execution_isolation::execution_isolation_artifact_ref(&plan)]
}

fn isolation_profile_violations(
    isolation_profile: &str,
    capability_policy: &Value,
) -> Vec<&'static str> {
    let mut violations = Vec::new();
    match isolation_profile {
        "read_only" => {
            push_policy_flag_violation(capability_policy, "network_access", &mut violations);
            push_policy_flag_violation(capability_policy, "filesystem_write", &mut violations);
            push_policy_flag_violation(capability_policy, "external_publish", &mut violations);
            push_policy_flag_violation(capability_policy, "credential_access", &mut violations);
        }
        "local_temp_workspace" | "local_worktree" => {
            push_policy_flag_violation(capability_policy, "external_publish", &mut violations);
            push_policy_flag_violation(capability_policy, "credential_access", &mut violations);
        }
        "local_current_workspace" | "remote_executor" => {}
        _ => violations.push("unknown_isolation_profile"),
    }
    violations
}

fn push_policy_flag_violation<'a>(
    capability_policy: &Value,
    key: &'a str,
    violations: &mut Vec<&'a str>,
) {
    if capability_policy.get(key).and_then(Value::as_bool) == Some(true) {
        violations.push(key);
    }
}

pub(super) fn preflight_permission_decision(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
    reason_code: &'static str,
    owner_layer: &'static str,
) -> Value {
    let canonical_skill = state.resolve_canonical_skill_name(normalized_skill);
    let action = normalized_action_arg(args);
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, &canonical_skill, args);
    let manifest = state.skill_manifest(&canonical_skill);
    let dry_run_observe_only =
        crate::execution_recipe::dry_run_observes_only_action(&canonical_skill, args);
    let risk_level = if dry_run_observe_only {
        Some(SkillRiskLevel::Low)
    } else {
        action_scoped_risk_level(state, &canonical_skill, action.as_deref())
            .or_else(|| manifest.as_ref().and_then(|value| value.risk_level))
    };
    let command_policy = run_cmd_command_policy(&canonical_skill, args, effect);
    let capability_policy =
        action_scoped_capability_policy(state, &canonical_skill, action.as_deref()).unwrap_or_else(
            || {
                json!({
                    "isolation_profile": null,
                    "network_access": null,
                    "filesystem_write": null,
                    "external_publish": null,
                    "credential_access": null,
                    "subprocess": null,
                    "package_install": null,
                    "privilege_escalation": null,
                })
            },
        );
    let sandbox_profile = sandbox_profile_token(&capability_policy, effect);
    let workspace_scope = permission_workspace_scope_summary(state, &canonical_skill, args);
    let risk_requires_confirmation = !dry_run_observe_only
        && state.skill_invocation_requires_confirmation_policy(&canonical_skill, Some(args));
    let needs_confirmation = state.skill_rt.tools_policy.approval_required(
        risk_requires_confirmation,
        false,
        effect.mutates
            || capability_policy
                .get("external_publish")
                .and_then(Value::as_bool)
                .unwrap_or(false),
    );
    let sandbox_denial_reason =
        crate::verifier::skill_sandbox_denial_reason(state, None, &canonical_skill, args);
    let sandbox_network = if capability_policy
        .get("network_access")
        .and_then(Value::as_bool)
        == Some(true)
    {
        crate::process_sandbox::ProcessNetworkPolicy::Inherit
    } else {
        crate::process_sandbox::ProcessNetworkPolicy::Deny
    };
    let sandbox_backend_diagnostics = crate::process_sandbox::sandbox_backend_diagnostics(
        state.skill_rt.tools_policy.sandbox_backend,
        state.skill_rt.tools_policy.sandbox_mode,
        sandbox_network,
    );
    let decision = crate::policy_decision::PolicyDecision::from_permission_flags(
        false,
        needs_confirmation,
        true,
        false,
    );
    let registry = state.get_skills_registry();
    let registry_policy = state
        .mcp_tool(&canonical_skill)
        .map(|tool| {
            json!({
                "available": true,
                "source": "mcp_config",
                "once_per_task": !tool.policy.idempotent,
                "dedup_scope": "args",
                "idempotent": tool.policy.idempotent,
            })
        })
        .or_else(|| {
            registry.as_ref().map(|registry| {
        json!({
            "available": true,
            "once_per_task": registry.resolved_once_per_task(&canonical_skill, action.as_deref()),
            "dedup_scope": registry
                .resolved_dedup_scope(&canonical_skill, action.as_deref())
                .as_token(),
            "idempotent": registry.resolved_idempotent(&canonical_skill, action.as_deref()),
        })
    })
        });
    json!({
        "schema_version": 1,
        "decision": decision.as_token(),
        "allowed": false,
        "needs_confirmation": needs_confirmation,
        "approval_policy": state.skill_rt.tools_policy.approval_policy_token(),
        "global_sandbox_mode": state.skill_rt.tools_policy.sandbox_mode_token(),
        "global_sandbox_backend": state.skill_rt.tools_policy.sandbox_backend_token(),
        "sandbox_backend_diagnostics": sandbox_backend_diagnostics,
        "sandbox_denial_reason": sandbox_denial_reason,
        "denied_by_policy": true,
        "dry_run_required": false,
        "external_provider_blocked": false,
        "reason_code": reason_code,
        "owner_layer": owner_layer,
        "risk_level": risk_level_token(risk_level),
        "canonical_skill": canonical_skill,
        "action": action,
        "action_effect": action_effect_token(effect),
        "observes": effect.observes,
        "mutates": effect.mutates,
        "validates": effect.validates,
        "sandbox_profile": sandbox_profile.clone(),
        "sandbox": sandbox_policy_summary(&capability_policy, &sandbox_profile, effect),
        "workspace_scope": workspace_scope,
        "command_policy": command_policy,
        "capability_policy": capability_policy,
        "registry_policy": registry_policy.unwrap_or_else(|| {
            json!({
                "available": false,
                "once_per_task": false,
                "dedup_scope": "args",
                "idempotent": false,
            })
        }),
    })
}

fn is_path_like_arg_key(key: &str) -> bool {
    let key = key.trim();
    matches!(
        key,
        "path" | "db_path" | "root" | "cwd" | "directory" | "dir"
    ) || key.ends_with("_path")
        || key.ends_with("_root")
}

fn looks_like_structured_runtime_observation(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) | Ok(Value::Array(_)) => true,
        _ => false,
    }
}

fn contains_structured_observation_in_path_arg(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            if is_path_like_arg_key(key) {
                return value
                    .as_str()
                    .is_some_and(looks_like_structured_runtime_observation);
            }
            contains_structured_observation_in_path_arg(value)
        }),
        Value::Array(items) => items
            .iter()
            .any(contains_structured_observation_in_path_arg),
        _ => false,
    }
}

pub(super) fn structured_observation_path_argument_error(
    normalized_skill: &str,
    exec_args: &Value,
) -> Option<String> {
    if !contains_structured_observation_in_path_arg(exec_args) {
        return None;
    }
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "invalid_args",
        "path argument contains a structured observation instead of one concrete path; select a single path from observed fields or ask for clarification when multiple candidates exist",
        None,
        Some(json!({
            "reason": "structured_observation_embedded_in_path_arg",
        })),
    ))
}

pub(super) struct PreflightFailureMetadata {
    pub(super) reason: &'static str,
    pub(super) error_kind: String,
    pub(super) retry_instruction: String,
}

fn structured_error_extra_string(
    structured: &crate::skills::StructuredSkillError,
    key: &str,
) -> Option<String> {
    structured
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn structured_error_extra_string_list(
    structured: &crate::skills::StructuredSkillError,
    key: &str,
) -> Vec<String> {
    structured
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn contract_policy_retry_instruction(
    structured: &crate::skills::StructuredSkillError,
) -> Option<String> {
    let decision = structured_error_extra_string(structured, "decision")?;
    let policy_decision = structured_error_extra_string(structured, "policy_decision")
        .unwrap_or_else(|| {
            crate::policy_decision::PolicyDecision::Deny
                .as_token()
                .to_string()
        });
    let action = structured_error_extra_string(structured, "action")
        .unwrap_or_else(|| "unknown_action".to_string());
    let contract = structured_error_extra_string(structured, "contract_match")
        .unwrap_or_else(|| "unknown_contract".to_string());
    let mut parts = vec![format!(
        "contract_policy_decision={decision};rejected_action={action};contract={contract}"
    )];
    parts.push(format!("policy_decision={policy_decision}"));
    let preferred = structured_error_extra_string_list(structured, "preferred_actions");
    if !preferred.is_empty() {
        parts.push(format!("preferred_action_refs={}", preferred.join("|")));
    }
    let expected_targets = structured_error_extra_string_list(structured, "expected_target_args");
    if !expected_targets.is_empty() {
        parts.push(format!(
            "required_target_args={}",
            expected_targets.join("|")
        ));
    }
    parts.push("retry_policy=no_repeat_rejected_action_without_contract_change".to_string());
    Some(parts.join(";"))
}

pub(super) fn preflight_failure_metadata(err: &str) -> PreflightFailureMetadata {
    let structured = crate::skills::parse_structured_skill_error(err);
    let error_kind = structured
        .as_ref()
        .map(|value| value.error_kind.clone())
        .unwrap_or_else(|| "invalid_args".to_string());
    if matches!(
        error_kind.as_str(),
        "contract_action_rejected" | "contract_arg_rejected"
    ) {
        let retry_instruction = structured
            .as_ref()
            .and_then(contract_policy_retry_instruction)
            .unwrap_or_else(|| {
                "contract_policy_decision=unavailable;retry_policy=choose_allowed_action_or_replan"
                    .to_string()
            });
        return PreflightFailureMetadata {
            reason: if error_kind == "contract_action_rejected" {
                "contract_action_rejected"
            } else {
                "contract_arg_rejected"
            },
            error_kind,
            retry_instruction,
        };
    }
    if error_kind == "isolation_policy_violation" {
        return PreflightFailureMetadata {
            reason: "isolation_policy_violation",
            error_kind,
            retry_instruction:
                "isolation_policy=choose_capability_matching_profile;retry_same_policy=false"
                    .to_string(),
        };
    }
    if error_kind == "child_task_policy_violation" {
        return PreflightFailureMetadata {
            reason: "child_task_policy_violation",
            error_kind,
            retry_instruction:
                "child_task_policy=use_declared_capability_and_permission_profile;retry_same_policy=false"
                    .to_string(),
        };
    }
    if structured
        .as_ref()
        .and_then(|value| structured_error_extra_string(value, "reason"))
        .as_deref()
        == Some("structured_observation_embedded_in_path_arg")
    {
        return PreflightFailureMetadata {
            reason: "structured_observation_embedded_in_path_arg",
            error_kind,
            retry_instruction:
                "path_arg_policy=concrete_observed_path_or_clarify;structured_observation_embedded=false"
                    .to_string(),
        };
    }
    PreflightFailureMetadata {
        reason: "unresolved_runtime_placeholder",
        error_kind,
        retry_instruction:
            "placeholder_policy=resolve_from_observed_value_or_synthesize_or_pipeline;retry_same_placeholder=false"
                .to_string(),
    }
}

pub(super) fn handle_preflight_argument_failure(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    normalized_skill: &str,
    classification_args: &Value,
    err: &str,
    action_trace_kind: &str,
) -> SkillActionOutcome {
    let error_observation = super::skill_error_observation_or_raw(normalized_skill, err);
    let progress_error = super::skill_error_progress_token(normalized_skill, err);
    let metadata = preflight_failure_metadata(err);
    attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        normalized_skill,
        &format!("preflight=rejected_{}", metadata.reason),
        crate::executor::StepExecutionStatus::Error,
        "",
        Some(metadata.error_kind.as_str()),
        &error_observation,
        Some(metadata.retry_instruction.as_str()),
    );
    let effect = crate::execution_recipe::classify_skill_action_effect(
        state,
        normalized_skill,
        classification_args,
    );
    crate::execution_recipe::apply_action_effect_failure(&mut loop_state.execution_recipe, effect);
    maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        false,
        &error_observation,
    );
    register_failed_step_output(
        loop_state,
        global_step,
        step_in_round,
        &format!("skill.{normalized_skill}"),
        &format!("skill({normalized_skill})"),
        &error_observation,
    );
    let now = crate::now_ts_u64();
    let step_execution = crate::executor::StepExecutionResult {
        step_id: format!("step_{global_step}"),
        skill: normalized_skill.to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(err.to_string()),
        started_at: now,
        finished_at: now,
    };
    loop_state
        .capability_results
        .push(crate::capability_result::envelope_for_step_execution(
            normalized_skill,
            classification_args,
            &step_execution,
            None,
        ));
    loop_state
        .executed_step_results
        .push(step_execution.clone());
    super::log_step_journal_summary(
        task,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        loop_state
            .execution_recipe
            .is_active()
            .then(|| loop_state.execution_recipe.phase_summary_line())
            .as_deref(),
        &step_execution,
    );
    loop_state.history_compact.push(format!(
        "round={} step={} skill={} rejected_{}",
        loop_state.round_no, step_in_round, normalized_skill, metadata.reason
    ));
    super::publish_failure_recovery_progress(
        state,
        task,
        loop_state,
        step_in_round,
        normalized_skill,
        &progress_error,
        "recoverable_failure_continue_round",
    );
    info!(
        "executor_preflight_arg_rejected task_id={} round={} step={} type={} skill={} reason={} error_kind={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        normalized_skill,
        metadata.reason,
        metadata.error_kind
    );
    SkillActionOutcome {
        ended_with_user_visible_output: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        continue_in_round: false,
    }
}
