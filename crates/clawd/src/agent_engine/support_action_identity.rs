/// Append to final delivery only. This is the only path that feeds user-visible result. No progress publish.
pub(crate) fn append_delivery_message(
    task_id: &str,
    delivery_messages: &mut Vec<String>,
    message: String,
) {
    let message = crate::visible_text::sanitize_user_visible_text(&message);
    delivery_messages.push(message.clone());
    info!(
        "delivery appended task_id={} len={} content={}",
        task_id,
        delivery_messages.len(),
        crate::truncate_for_log(&message)
    );
}

pub(super) fn action_fingerprint(state: &AppState, action: &AgentAction) -> String {
    match action {
        AgentAction::CallTool { tool, args } => {
            let normalized_skill = state
                .resolve_canonical_skill_name(tool.trim())
                .to_ascii_lowercase();
            let normalized_args = normalize_args_for_fingerprint(state, &normalized_skill, args);
            format!(
                "skill:{}:{}",
                normalized_skill,
                canonical_json_string(&normalized_args)
            )
        }
        AgentAction::CallSkill { skill, args } => {
            let normalized_skill = state
                .resolve_canonical_skill_name(skill)
                .to_ascii_lowercase();
            let normalized_args = normalize_args_for_fingerprint(state, &normalized_skill, args);
            format!(
                "skill:{}:{}",
                normalized_skill,
                canonical_json_string(&normalized_args)
            )
        }
        AgentAction::Respond { content } => {
            format!("respond:{}", content.trim().to_ascii_lowercase())
        }
        AgentAction::SynthesizeAnswer { evidence_refs } => format!(
            "synthesize_answer:{}",
            evidence_refs
                .iter()
                .map(|item| item.trim().to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(",")
        ),
        AgentAction::CallCapability { capability, args } => {
            let normalized = capability.trim().to_ascii_lowercase();
            let normalized_args = normalize_args_for_fingerprint(state, &normalized, args);
            format!(
                "capability:{}:{}",
                normalized,
                canonical_json_string(&normalized_args)
            )
        }
        AgentAction::Think { .. } => "think".to_string(),
    }
}

pub(super) fn action_fingerprint_for_policy(
    state: &AppState,
    policy: &AgentLoopGuardPolicy,
    action: &AgentAction,
) -> String {
    if !policy.registry_idempotency_guard_enabled() {
        return action_fingerprint(state, action);
    }
    let resolved_action = resolved_registry_action_for_policy(state, action);
    let policy_action = resolved_action.as_ref().unwrap_or(action);
    let Some((skill_name, args)) = action_skill_and_args(policy_action) else {
        return action_fingerprint(state, action);
    };
    let normalized_skill = state
        .resolve_canonical_skill_name(skill_name)
        .to_ascii_lowercase();
    let action_token = registry_action_token_from_args(args);
    let Some(registry) = state.get_skills_registry() else {
        return action_fingerprint(state, action);
    };
    let dedup_scope = registry.resolved_dedup_scope(&normalized_skill, action_token.as_deref());
    match dedup_scope {
        claw_core::skill_registry::RegistryDedupScope::Action => {
            if run_command_action_uses_args_fingerprint(
                &normalized_skill,
                action_token.as_deref(),
                args,
            ) {
                return action_fingerprint(state, policy_action);
            }
            return format!(
                "skill:{}:action:{}",
                normalized_skill,
                action_token.unwrap_or_else(|| "_default".to_string())
            );
        }
        claw_core::skill_registry::RegistryDedupScope::Resource => {
            let fields = registry.resolved_dedup_fields(&normalized_skill, action_token.as_deref());
            let resource = fields
                .iter()
                .filter_map(|field| args.get(field).map(|value| (field.clone(), value.clone())))
                .collect::<serde_json::Map<String, Value>>();
            if resource.is_empty() {
                return action_fingerprint(state, policy_action);
            }
            return format!(
                "skill:{}:action:{}:resource:{}",
                normalized_skill,
                action_token.unwrap_or_else(|| "_default".to_string()),
                canonical_json_string(&Value::Object(resource))
            );
        }
        claw_core::skill_registry::RegistryDedupScope::Args => {}
    }
    action_fingerprint(state, policy_action)
}

pub(super) fn registry_idempotency_guard_attribution(
    state: &AppState,
    policy: &AgentLoopGuardPolicy,
    action: &AgentAction,
    fingerprint: &str,
    reason_code: &str,
    repeat_count: Option<usize>,
    limit: Option<usize>,
) -> Option<crate::task_journal::TaskJournalRolloutAttribution> {
    if !policy.registry_idempotency_guard_enabled() {
        return None;
    }
    let resolved_action = resolved_registry_action_for_policy(state, action);
    let policy_action = resolved_action.as_ref().unwrap_or(action);
    let (skill_name, args) = action_skill_and_args(policy_action)?;
    let normalized_skill = state
        .resolve_canonical_skill_name(skill_name)
        .to_ascii_lowercase();
    let action_token = registry_action_token_from_args(args);
    let registry = state.get_skills_registry()?;
    let once_per_task = registry.resolved_once_per_task(&normalized_skill, action_token.as_deref());
    let dedup_scope = registry.resolved_dedup_scope(&normalized_skill, action_token.as_deref());
    if !once_per_task && dedup_scope == claw_core::skill_registry::RegistryDedupScope::Args {
        return None;
    }
    if run_command_action_uses_args_fingerprint(&normalized_skill, action_token.as_deref(), args) {
        return None;
    }
    Some(
        crate::task_journal::TaskJournalRolloutAttribution::registry_idempotency_guard_block(
            reason_code,
            normalized_skill,
            action_token,
            dedup_scope.as_token(),
            fingerprint,
            repeat_count,
            limit,
        ),
    )
}

fn resolved_registry_action_for_policy(
    state: &AppState,
    action: &AgentAction,
) -> Option<AgentAction> {
    if !matches!(action, AgentAction::CallCapability { .. }) {
        return None;
    }
    let resolved =
        crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
    (!matches!(resolved, AgentAction::CallCapability { .. })).then_some(resolved)
}

fn action_skill_and_args(action: &AgentAction) -> Option<(&str, &Value)> {
    match action {
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        _ => None,
    }
}

fn registry_action_token_from_args(args: &Value) -> Option<String> {
    args.get("action")
        .and_then(Value::as_str)
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
        .filter(|value| !value.is_empty())
}

fn run_command_action_uses_args_fingerprint(
    normalized_skill: &str,
    action_token: Option<&str>,
    args: &Value,
) -> bool {
    let is_run_command_action = normalized_skill == "run_cmd"
        || (normalized_skill == "system_basic" && action_token == Some("run_cmd"));
    if !is_run_command_action {
        return false;
    }
    args.get("command")
        .or_else(|| args.get("cmd"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|command| !command.is_empty())
}
