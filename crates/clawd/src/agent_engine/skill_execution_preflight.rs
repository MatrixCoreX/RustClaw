use claw_core::skill_registry::{PlannerCapabilityEffect, SkillRiskLevel};
use serde_json::{json, Value};
use tracing::info;

use super::{register_failed_step_output, AppState, ClaimedTask, LoopState, SkillActionOutcome};
use crate::agent_engine::{
    action_has_user_named_output_path_marker, attempt_ledger,
    maybe_publish_execution_recipe_phase_hint, CLAWD_LITERAL_COMMAND_ARG,
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

pub(super) fn contract_matrix_action_policy_error(
    state: &AppState,
    loop_state: &LoopState,
    normalized_skill: &str,
    classification_args: &Value,
) -> Option<String> {
    if matches!(
        normalized_skill,
        "synthesize_answer" | "respond" | "think" | "answer_verifier"
    ) {
        return None;
    }
    if let Some(err) = generated_media_path_run_cmd_policy_error(
        state,
        loop_state,
        normalized_skill,
        classification_args,
    ) {
        return Some(err);
    }
    let policy = crate::contract_matrix::action_policy_for_output_contract(
        loop_state.output_contract.as_ref(),
        normalized_skill,
        classification_args,
    )?;
    if policy.is_allowed() {
        return None;
    }
    if active_ops_recipe_allows_mutation_despite_contract(
        state,
        loop_state,
        normalized_skill,
        classification_args,
        policy.decision,
    ) {
        info!(
            "preflight_keep_active_ops_recipe_mutation_despite_contract skill={} action={} contract={} phase={}",
            normalized_skill,
            policy.action_key,
            policy.contract_match,
            loop_state.execution_recipe.phase.as_str()
        );
        return None;
    }
    if runtime_async_job_start_allows_run_cmd_despite_contract(
        normalized_skill,
        classification_args,
        policy.decision,
    ) {
        info!(
            "preflight_keep_runtime_async_job_start_despite_contract skill={} action={} contract={}",
            normalized_skill, policy.action_key, policy.contract_match
        );
        return None;
    }
    if registry_action_can_extend_summary_contract(
        state,
        normalized_skill,
        classification_args,
        &policy.contract_match,
    ) {
        info!(
            "preflight_keep_registry_non_mutating_action skill={} action={} contract={}",
            normalized_skill, policy.action_key, policy.contract_match
        );
        return None;
    }
    if action_has_user_named_output_path_marker(classification_args) {
        return None;
    }
    let mut error_text = format!(
        "action `{}` is rejected by contract `{}` ({})",
        policy.action_key,
        policy.contract_match,
        policy.decision.as_str()
    );
    if !policy.preferred_actions.is_empty() {
        error_text.push_str(&format!(
            "; prefer action(s): {}",
            policy.preferred_actions.join(", ")
        ));
    }
    let evidence_expression = policy
        .evidence_expression
        .to_trace_json(&policy.required_evidence);
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "contract_action_rejected",
        &error_text,
        None,
        Some(json!({
            "reason_code": "contract_action_rejected",
            "failure_attribution": crate::contract_matrix::FailureAttribution::ContractGap.as_str(),
            "decision": policy.decision.as_str(),
            "action": policy.action_key,
            "original_action_ref": policy.original_action_ref,
            "replacement_action_ref": policy.replacement_action_ref,
            "contract_repair_source": policy.contract_repair_source,
            "preferred_replacement_reason_code": policy.preferred_replacement_reason_code,
            "contract_match": policy.contract_match,
            "required_evidence": policy.required_evidence,
            "preferred_actions": policy.preferred_actions,
            "evidence_expression": evidence_expression,
            "final_answer_shape": policy.final_answer_shape,
            "policy_mode": policy.policy_mode,
            "evidence_scope": policy.evidence_scope,
            "freshness": policy.freshness,
            "artifact_kind": policy.artifact_kind,
            "channel_visibility": policy.channel_visibility,
            "evidence_profile": policy.evidence_profile,
            "permission_decision": preflight_permission_decision(
                state,
                normalized_skill,
                classification_args,
                "contract_action_rejected",
                "contract_matrix_preflight",
            ),
        })),
    ))
}

fn runtime_async_job_start_allows_run_cmd_despite_contract(
    normalized_skill: &str,
    classification_args: &Value,
    decision: crate::contract_matrix::ActionPolicyDecision,
) -> bool {
    if !normalized_skill.eq_ignore_ascii_case("run_cmd")
        || !matches!(
            decision,
            crate::contract_matrix::ActionPolicyDecision::RejectedForbidden
                | crate::contract_matrix::ActionPolicyDecision::RejectedNotAllowed
        )
        || classification_args
            .get("async_start")
            .and_then(Value::as_bool)
            != Some(true)
        || classification_args
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return false;
    }
    positive_bounded_i64_arg(classification_args, "poll_after_seconds", 1, 86_400)
        && positive_bounded_i64_arg(classification_args, "expires_in_seconds", 1, 604_800)
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
    if output_contract.semantic_kind != crate::OutputSemanticKind::GeneratedFilePathReport
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
            "failure_attribution": crate::contract_matrix::FailureAttribution::ModelError.as_str(),
            "decision": crate::contract_matrix::ActionPolicyDecision::RejectedNotAllowed.as_str(),
            "policy_decision": crate::policy_decision::PolicyDecision::from_contract_action_policy(
                crate::contract_matrix::ActionPolicyDecision::RejectedNotAllowed
            ).as_token(),
            "action": "run_cmd",
            "original_action_ref": "run_cmd",
            "contract_match": crate::OutputSemanticKind::GeneratedFilePathReport.as_str(),
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

fn registry_declares_non_mutating_planner_action(
    state: &AppState,
    canonical_skill: &str,
    args: &Value,
) -> bool {
    let Some(action) = normalized_action_arg(args) else {
        return false;
    };
    state
        .skill_manifest(canonical_skill)
        .is_some_and(|manifest| {
            manifest.planner_capabilities.into_iter().any(|mapping| {
                mapping
                    .action
                    .as_deref()
                    .map(|value| {
                        value
                            .trim()
                            .to_ascii_lowercase()
                            .chars()
                            .map(|ch| {
                                if matches!(ch, '-' | ' ' | '.') {
                                    '_'
                                } else {
                                    ch
                                }
                            })
                            .collect::<String>()
                    })
                    .is_some_and(|mapped| mapped == action)
                    && matches!(
                        mapping.effect,
                        Some(PlannerCapabilityEffect::Observe | PlannerCapabilityEffect::Validate)
                    )
            })
        })
}

fn registry_action_can_extend_summary_contract(
    state: &AppState,
    canonical_skill: &str,
    args: &Value,
    contract_match: &str,
) -> bool {
    contract_match == "command_output_summary"
        && registry_declares_non_mutating_planner_action(state, canonical_skill, args)
}

fn action_scoped_risk_level(
    state: &AppState,
    canonical_skill: &str,
    action: Option<&str>,
) -> Option<SkillRiskLevel> {
    let action = action?;
    state.skill_manifest(canonical_skill).and_then(|manifest| {
        manifest
            .planner_capabilities
            .into_iter()
            .find(|mapping| mapping.action.as_deref() == Some(action))
            .and_then(|mapping| mapping.risk_level)
    })
}

fn action_scoped_capability_policy(
    state: &AppState,
    canonical_skill: &str,
    action: Option<&str>,
) -> Option<Value> {
    state.skill_manifest(canonical_skill).and_then(|manifest| {
        let capabilities = manifest.planner_capabilities;
        capabilities
            .iter()
            .find(|mapping| mapping.action.as_deref() == action)
            .or_else(|| {
                if capabilities.len() == 1 {
                    capabilities.first()
                } else {
                    None
                }
            })
            .map(|mapping| {
                json!({
                    "isolation_profile": mapping
                        .isolation_profile
                        .map(|value| value.as_token()),
                    "network_access": mapping.network_access,
                    "filesystem_write": mapping.filesystem_write,
                    "external_publish": mapping.external_publish,
                    "credential_access": mapping.credential_access,
                })
            })
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
            "failure_attribution": crate::contract_matrix::FailureAttribution::PermissionDenied.as_str(),
            "decision": crate::contract_matrix::ActionPolicyDecision::RejectedForbidden.as_str(),
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

fn package_manager_dry_run_install_action(canonical_skill: &str, args: &Value) -> bool {
    if canonical_skill != "package_manager" {
        return false;
    }
    if args.get("dry_run").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    matches!(
        normalized_action_arg(args).as_deref(),
        Some("install" | "uninstall" | "smart_install")
    )
}

fn preflight_permission_decision(
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
    let risk_level = if package_manager_dry_run_install_action(&canonical_skill, args) {
        Some(SkillRiskLevel::Low)
    } else {
        action_scoped_risk_level(state, &canonical_skill, action.as_deref())
            .or_else(|| manifest.as_ref().and_then(|value| value.risk_level))
    };
    let command_policy = run_cmd_command_policy(&canonical_skill, args, effect);
    let capability_policy =
        action_scoped_capability_policy(state, &canonical_skill, action.as_deref());
    let needs_confirmation =
        state.skill_invocation_requires_confirmation_policy(&canonical_skill, Some(args));
    let decision = crate::policy_decision::PolicyDecision::from_permission_flags(
        false,
        needs_confirmation,
        true,
        false,
    );
    let registry = state.get_skills_registry();
    let registry_policy = registry.as_ref().map(|registry| {
        json!({
            "available": true,
            "once_per_task": registry.resolved_once_per_task(&canonical_skill, action.as_deref()),
            "dedup_scope": registry
                .resolved_dedup_scope(&canonical_skill, action.as_deref())
                .as_token(),
            "idempotent": registry.resolved_idempotent(&canonical_skill, action.as_deref()),
        })
    });
    json!({
        "schema_version": 1,
        "decision": decision.as_token(),
        "allowed": false,
        "needs_confirmation": needs_confirmation,
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
        "command_policy": command_policy,
        "capability_policy": capability_policy.unwrap_or_else(|| {
            json!({
                "isolation_profile": null,
                "network_access": null,
                "filesystem_write": null,
                "external_publish": null,
                "credential_access": null,
            })
        }),
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

fn active_ops_recipe_allows_mutation_despite_contract(
    state: &AppState,
    loop_state: &LoopState,
    normalized_skill: &str,
    args: &Value,
    policy_decision: crate::contract_matrix::ActionPolicyDecision,
) -> bool {
    if policy_decision != crate::contract_matrix::ActionPolicyDecision::RejectedNotAllowed {
        return false;
    }
    let recipe = loop_state.execution_recipe;
    if !matches!(
        recipe.kind,
        crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
    ) || !matches!(
        recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Apply
            | crate::execution_recipe::ExecutionRecipePhase::Repair
    ) {
        return false;
    }
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args);
    effect.mutates
        && !crate::execution_recipe::action_conflicts_with_recipe_target_scope(
            recipe,
            state,
            normalized_skill,
            args,
        )
}

pub(super) fn contract_matrix_arg_policy_error(
    loop_state: &LoopState,
    normalized_skill: &str,
    exec_args: &Value,
) -> Option<String> {
    let policy = crate::contract_matrix::arg_policy_decision(
        loop_state.output_contract.as_ref(),
        normalized_skill,
        exec_args,
    )?;
    if policy.is_allowed()
        || policy.decision == crate::contract_matrix::ArgPolicyDecision::DeferredTemplateArg
    {
        return None;
    }
    let mut error_text = format!(
        "action `{}` is missing target binding required by contract `{}`",
        policy.action_key, policy.contract_match
    );
    if !policy.expected_target_args.is_empty() {
        error_text.push_str(&format!(
            "; expected target arg(s): {}",
            policy.expected_target_args.join(", ")
        ));
    }
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "contract_arg_rejected",
        &error_text,
        None,
        Some(json!({
            "reason_code": "contract_arg_rejected",
            "failure_attribution": crate::contract_matrix::FailureAttribution::ModelError.as_str(),
            "decision": policy.decision.as_str(),
            "policy_decision": crate::policy_decision::PolicyDecision::from_contract_arg_policy(
                policy.decision
            ).as_token(),
            "action": policy.action_key,
            "contract_match": policy.contract_match,
            "required_evidence": policy.required_evidence,
            "missing_target_args": policy.missing_target_args,
            "expected_target_args": policy.expected_target_args,
            "final_answer_shape": policy.final_answer_shape,
            "policy_mode": policy.policy_mode,
            "evidence_scope": policy.evidence_scope,
            "freshness": policy.freshness,
            "artifact_kind": policy.artifact_kind,
            "channel_visibility": policy.channel_visibility,
            "evidence_profile": policy.evidence_profile,
        })),
    ))
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
        parts.push(format!("preferred_actions={}", preferred.join("|")));
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
    let user_visible_err = crate::skills::normalize_skill_error_for_user(normalized_skill, err);
    let metadata = preflight_failure_metadata(err);
    attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        normalized_skill,
        &format!("preflight=rejected_{}", metadata.reason),
        crate::executor::StepExecutionStatus::Error,
        "",
        Some(metadata.error_kind.as_str()),
        &user_visible_err,
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
        &user_visible_err,
    );
    register_failed_step_output(
        loop_state,
        global_step,
        step_in_round,
        &format!("skill.{normalized_skill}"),
        &format!("skill({normalized_skill})"),
        &user_visible_err,
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
        &user_visible_err,
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
