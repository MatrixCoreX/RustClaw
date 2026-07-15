use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoopStateCheckpointSeedReport {
    pub(crate) checkpoint_id: String,
    pub(crate) resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint,
    pub(crate) restored_round: usize,
    pub(crate) restored_step: usize,
    pub(crate) restored_tool_calls: usize,
    pub(crate) completed_side_effect_count: usize,
    pub(crate) observation_count: usize,
}

pub(crate) fn seed_loop_state_from_task_checkpoint(
    loop_state: &mut LoopState,
    checkpoint: &crate::task_lifecycle::TaskCheckpoint,
) -> LoopStateCheckpointSeedReport {
    let restored_round = checkpoint.budget.round as usize;
    let restored_step = checkpoint.budget.step as usize;
    let restored_tool_calls = checkpoint.budget.tool_calls as usize;
    loop_state.round_no = loop_state.round_no.max(restored_round);
    loop_state.total_steps_executed = loop_state.total_steps_executed.max(restored_step);
    loop_state.tool_calls_total = loop_state.tool_calls_total.max(restored_tool_calls);

    let mut completed_side_effect_count = 0usize;
    for fingerprint in &checkpoint.completed_side_effect_refs {
        let fingerprint = fingerprint.trim();
        if fingerprint.is_empty() {
            continue;
        }
        completed_side_effect_count += 1;
        loop_state
            .successful_action_fingerprints
            .entry(fingerprint.to_string())
            .or_insert(1);
    }

    if !checkpoint.observations.is_empty() {
        loop_state.has_tool_or_skill_output = true;
    }
    let changed_files = checkpoint
        .artifact_refs
        .iter()
        .filter_map(|artifact_ref| artifact_ref.trim().strip_prefix("changed_file:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if let Some(last) = changed_files.last() {
        loop_state.last_written_file_path = Some(last.clone());
        loop_state
            .output_vars
            .insert("last_written_file_path".to_string(), last.clone());
    }
    if !changed_files.is_empty() {
        if let Ok(serialized) = serde_json::to_string(&changed_files) {
            loop_state.output_vars.insert(
                "agent_loop.resume_changed_files_json".to_string(),
                serialized,
            );
        }
    }
    loop_state.task_checkpoint = Some(checkpoint.to_machine_json());
    let resume_entrypoint = checkpoint_resume_entrypoint_token(&checkpoint.resume_entrypoint);
    loop_state.output_vars.insert(
        "agent_loop.resume_checkpoint_id".to_string(),
        checkpoint.checkpoint_id.clone(),
    );
    loop_state.output_vars.insert(
        "agent_loop.resume_entrypoint".to_string(),
        resume_entrypoint.to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.resume_completed_side_effect_count".to_string(),
        completed_side_effect_count.to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.resume_observation_count".to_string(),
        checkpoint.observations.len().to_string(),
    );
    if let Some(attempt_ledger) = checkpoint
        .attempt_ledger
        .as_ref()
        .filter(|value| value.is_array() || value.is_object())
    {
        loop_state.output_vars.insert(
            "agent_loop.resume_attempt_ledger_present".to_string(),
            "true".to_string(),
        );
        if let Ok(snapshot) = serde_json::to_string(attempt_ledger) {
            loop_state
                .history_compact
                .push(format!("checkpoint_attempt_ledger_json={snapshot}"));
        }
    }

    loop_state.history_compact.push(format!(
        "checkpoint_resume checkpoint_id={} entrypoint={} round={} step={} tool_calls={} side_effects={} observations={}",
        checkpoint.checkpoint_id,
        resume_entrypoint,
        restored_round,
        restored_step,
        restored_tool_calls,
        completed_side_effect_count,
        checkpoint.observations.len()
    ));

    LoopStateCheckpointSeedReport {
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        resume_entrypoint: checkpoint.resume_entrypoint.clone(),
        restored_round,
        restored_step,
        restored_tool_calls,
        completed_side_effect_count,
        observation_count: checkpoint.observations.len(),
    }
}

fn checkpoint_resume_entrypoint_token(
    entrypoint: &crate::task_lifecycle::ResumeEntrypoint,
) -> &'static str {
    match entrypoint {
        crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound => "next_planner_round",
        crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob => "poll_async_job",
        crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput => "await_user_input",
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize => "verify_and_finalize",
    }
}

pub(super) fn seed_loop_state_from_agent_context(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(ctx) = agent_run_context else {
        return;
    };
    if let Some(path) = ctx
        .auto_locator_path
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), path.to_string());
    }
    if let Some(route) = ctx.route_result.as_ref() {
        loop_state.output_vars.insert(
            "route_locator_kind".to_string(),
            route.output_contract.locator_kind.as_str().to_string(),
        );
        loop_state.output_contract = Some(route.effective_output_contract());
        loop_state.route_policy_context = Some(route.clone());
    }
    if let Some(cross_turn_ctx) = ctx
        .cross_turn_recent_execution_context
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "<none>")
    {
        loop_state.output_vars.insert(
            "cross_turn_recent_execution_context".to_string(),
            cross_turn_ctx.to_string(),
        );
    }
    let alias_bindings = session_alias_bindings_for_loop_seed(ctx);
    let alias_request_texts = [
        ctx.original_user_request.as_deref(),
        ctx.user_request
            .as_deref()
            .map(alias_mention_request_surface),
    ];
    let mut required_alias_targets = Vec::new();
    for alias_request_text in alias_request_texts.into_iter().flatten() {
        required_alias_targets.extend(
            crate::conversation_state::alias_bindings_mentioned_in_prompt(
                &alias_bindings,
                alias_request_text,
            )
            .into_iter()
            .filter_map(|binding| {
                let target = binding.target.trim();
                (!target.is_empty()).then_some(target.to_string())
            }),
        );
    }
    required_alias_targets.sort();
    required_alias_targets.dedup();
    if !required_alias_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&required_alias_targets) {
            loop_state
                .output_vars
                .insert("required_session_alias_targets".to_string(), encoded);
        }
    }
    let active_bound_targets = active_bound_targets_for_loop_seed(ctx);
    if !active_bound_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&active_bound_targets) {
            loop_state
                .output_vars
                .insert("active_bound_targets".to_string(), encoded);
        }
    }
    let file_delivery_target_candidates = file_delivery_target_candidates_for_loop_seed(ctx);
    if !file_delivery_target_candidates.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&file_delivery_target_candidates) {
            loop_state
                .output_vars
                .insert("file_delivery_target_candidates".to_string(), encoded);
        }
    }
    let active_listing_bound_targets = active_listing_bound_targets_for_loop_seed(ctx);
    if !active_listing_bound_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&active_listing_bound_targets) {
            loop_state
                .output_vars
                .insert("active_listing_bound_targets".to_string(), encoded);
        }
    }
    let current_workspace_scalar_count_targets =
        current_workspace_scalar_count_targets_for_loop_seed(ctx);
    if !current_workspace_scalar_count_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&current_workspace_scalar_count_targets) {
            loop_state.output_vars.insert(
                "current_workspace_scalar_count_targets".to_string(),
                encoded,
            );
        }
    }
    let active_plan_file_targets = active_plan_file_targets_for_loop_seed(ctx);
    if !active_plan_file_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&active_plan_file_targets) {
            loop_state
                .output_vars
                .insert("active_plan_file_targets".to_string(), encoded);
        }
    }
    if boundary_observation_needs_clarify_for_loop_seed(ctx) {
        loop_state.boundary_observation_needs_clarify = true;
        loop_state.output_vars.insert(
            "agent_loop.boundary_observation_needs_clarify".to_string(),
            "true".to_string(),
        );
    }
    if pending_user_boundary_present_for_loop_seed(ctx) {
        loop_state.pending_user_boundary_present = true;
        loop_state.output_vars.insert(
            "agent_loop.pending_user_boundary_present".to_string(),
            "true".to_string(),
        );
    }
    let current_request_locator_evidence = current_request_locator_evidence_for_loop_seed(ctx);
    if !current_request_locator_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&current_request_locator_evidence) {
            loop_state
                .output_vars
                .insert("current_request_locator_evidence".to_string(), encoded);
        }
    }
    let current_request_resolved_workspace_child_targets =
        current_request_resolved_workspace_child_targets(&current_request_locator_evidence);
    if !current_request_resolved_workspace_child_targets.is_empty() {
        if let Ok(encoded) =
            serde_json::to_string(&current_request_resolved_workspace_child_targets)
        {
            loop_state.output_vars.insert(
                "current_request_resolved_workspace_child_targets".to_string(),
                encoded,
            );
        }
    }
    let default_main_config_contract_evidence =
        default_main_config_contract_evidence_for_loop_seed(ctx);
    if !default_main_config_contract_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&default_main_config_contract_evidence) {
            loop_state
                .output_vars
                .insert("default_main_config_contract_evidence".to_string(), encoded);
        }
        if let Some(logical_path) =
            first_string_field(&default_main_config_contract_evidence, "logical_path")
        {
            loop_state.output_vars.insert(
                "default_main_config_contract_logical_path".to_string(),
                logical_path,
            );
        }
        if let Some(workspace_path) =
            first_string_field(&default_main_config_contract_evidence, "workspace_path")
        {
            loop_state.output_vars.insert(
                "default_main_config_contract_workspace_path".to_string(),
                workspace_path,
            );
        }
    }
    let registry_capability_contract_evidence =
        registry_capability_contract_evidence_for_loop_seed(ctx);
    if !registry_capability_contract_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&registry_capability_contract_evidence) {
            loop_state
                .output_vars
                .insert("registry_capability_contract_evidence".to_string(), encoded);
        }
        let refs = registry_capability_contract_refs(&registry_capability_contract_evidence);
        if !refs.is_empty() {
            if let Ok(encoded) = serde_json::to_string(&refs) {
                loop_state
                    .output_vars
                    .insert("registry_capability_contract_refs".to_string(), encoded);
            }
        }
    }
    let contract_repair_candidate_evidence = contract_repair_candidate_evidence_for_loop_seed(ctx);
    if !contract_repair_candidate_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&contract_repair_candidate_evidence) {
            loop_state
                .output_vars
                .insert("contract_repair_candidate_evidence".to_string(), encoded);
        }
    }
    let pre_loop_clarify_candidates = pre_loop_clarify_candidates_for_loop_seed(ctx);
    if !pre_loop_clarify_candidates.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&pre_loop_clarify_candidates) {
            loop_state
                .output_vars
                .insert("pre_loop_clarify_candidates".to_string(), encoded);
        }
    }
    if let Some(spec) = ctx.execution_recipe_hint {
        loop_state.output_vars.insert(
            "route_execution_recipe_kind".to_string(),
            spec.kind.as_str().to_string(),
        );
        loop_state.output_vars.insert(
            "route_execution_recipe_profile".to_string(),
            spec.profile.as_str().to_string(),
        );
        loop_state.output_vars.insert(
            "route_execution_recipe_target_scope".to_string(),
            spec.target_scope.as_str().to_string(),
        );
    }
    if let Some(plan_hint) = ctx.execution_recipe_plan_hint.as_ref() {
        if !plan_hint.kind.trim().is_empty() {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_kind".to_string(),
                plan_hint.kind.trim().to_string(),
            );
        }
        if let Some(command) = plan_hint
            .command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_command".to_string(),
                command.to_string(),
            );
        }
        if let Some(mode) = plan_hint
            .execution_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_execution_mode".to_string(),
                mode.to_string(),
            );
        }
        if let Some(adapter_kind) = plan_hint
            .async_adapter_kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_async_adapter_kind".to_string(),
                adapter_kind.to_string(),
            );
        }
    }
}

pub(crate) fn seed_loop_state_for_agent_run(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
) -> Option<LoopStateCheckpointSeedReport> {
    let checkpoint_seed_report = resume_checkpoint
        .map(|checkpoint| seed_loop_state_from_task_checkpoint(loop_state, checkpoint));
    seed_loop_state_from_agent_context(loop_state, agent_run_context);
    checkpoint_seed_report
}

fn alias_mention_request_surface(text: &str) -> &str {
    text.split("### SESSION_ALIAS_BINDINGS")
        .next()
        .unwrap_or(text)
}

fn session_alias_bindings_for_loop_seed(
    ctx: &AgentRunContext,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    let mut bindings = ctx.session_alias_bindings.clone();
    if let Some(summary) = ctx.context_bundle_summary.as_deref() {
        bindings.extend(session_alias_bindings_from_context_summary(summary));
    }
    let mut seen = std::collections::BTreeSet::new();
    bindings.retain(|binding| {
        let alias = binding.alias.trim();
        let target = binding.target.trim();
        if alias.is_empty() || target.is_empty() {
            return false;
        }
        seen.insert((alias.to_string(), target.to_string()))
    });
    bindings
}

pub(in crate::agent_engine) fn session_alias_bindings_from_context_summary(
    summary: &str,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    let mut out = session_alias_bindings_from_context_alias_block(summary);
    out.extend(session_alias_bindings_from_boundary_observation_blocks(
        summary,
    ));
    out
}

fn session_alias_bindings_from_context_alias_block(
    summary: &str,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    let marker = "### SESSION_ALIAS_BINDINGS";
    let Some((_, tail)) = summary.split_once(marker) else {
        return Vec::new();
    };
    let block = tail.split("\n### ").next().unwrap_or(tail);
    let mut current_alias: Option<String> = None;
    let mut out = Vec::new();
    for line in block.lines() {
        let trimmed = line.trim();
        let alias = trimmed
            .strip_prefix("- alias:")
            .or_else(|| trimmed.strip_prefix("alias:"))
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(alias) = alias {
            current_alias = Some(alias.to_string());
            continue;
        }
        let target = trimmed
            .strip_prefix("target:")
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let (Some(alias), Some(target)) = (current_alias.take(), target) {
            out.push(crate::conversation_state::SessionAliasBinding {
                alias,
                target: target.to_string(),
                updated_at_ts: 0,
            });
        }
    }
    out
}

fn session_alias_bindings_from_boundary_observation_blocks(
    summary: &str,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(bindings) = value
            .get("session_alias_bindings")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for binding in bindings {
            let alias = binding
                .get("alias")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let target = binding
                .get("target")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let (Some(alias), Some(target)) = (alias, target) {
                out.push(crate::conversation_state::SessionAliasBinding {
                    alias: alias.to_string(),
                    target: target.to_string(),
                    updated_at_ts: 0,
                });
            }
        }
    }
    out
}

fn active_bound_targets_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(active_bound_targets_from_boundary_observation_blocks(
            summary,
        ));
    }
    targets.sort();
    targets.dedup();
    targets
}

fn file_delivery_target_candidates_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(file_delivery_target_candidates_from_boundary_observation_blocks(summary));
    }
    targets.sort();
    targets.dedup();
    targets
}

fn active_listing_bound_targets_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(active_listing_bound_targets_from_boundary_observation_blocks(summary));
    }
    targets.sort();
    targets.dedup();
    targets
}

fn current_workspace_scalar_count_targets_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(
            current_workspace_scalar_count_targets_from_boundary_observation_blocks(summary),
        );
    }
    targets.sort();
    targets.dedup();
    targets
}

fn current_request_locator_evidence_for_loop_seed(ctx: &AgentRunContext) -> Vec<Value> {
    let mut evidence = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        evidence.extend(current_request_locator_evidence_from_boundary_observation_blocks(summary));
    }
    evidence.sort_by_key(|value| serde_json::to_string(value).unwrap_or_default());
    evidence.dedup_by(|left, right| left == right);
    evidence
}

fn current_request_locator_evidence_from_boundary_observation_blocks(summary: &str) -> Vec<Value> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(locator) = value
            .get("current_request_locator")
            .and_then(Value::as_object)
        else {
            continue;
        };
        let has_concrete_surface = locator
            .get("has_concrete_surface")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let has_resolved_workspace_child = locator
            .get("resolved_workspace_child")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|target| !target.is_empty());
        let has_explicit_locator_hints = locator
            .get("explicit_locator_hints")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty());
        if has_concrete_surface || has_resolved_workspace_child || has_explicit_locator_hints {
            out.push(Value::Object(locator.clone()));
        }
    }
    out
}

fn current_request_resolved_workspace_child_targets(evidence: &[Value]) -> Vec<String> {
    let mut targets = evidence
        .iter()
        .filter_map(|value| {
            value
                .get("resolved_workspace_child")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    targets
}

fn active_bound_targets_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(targets) = value.get("active_bound_targets").and_then(Value::as_array) else {
            continue;
        };
        for target_value in targets {
            let Some(target) = target_value
                .get("target")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                if let Some(ordered_targets) = target_value
                    .get("ordered_targets")
                    .and_then(Value::as_array)
                {
                    out.extend(
                        ordered_targets
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToString::to_string),
                    );
                }
                continue;
            };
            out.push(target.to_string());
            if let Some(ordered_targets) = target_value
                .get("ordered_targets")
                .and_then(Value::as_array)
            {
                out.extend(
                    ordered_targets
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToString::to_string),
                );
            }
        }
    }
    out
}

fn file_delivery_target_candidates_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(targets) = value
            .get("file_delivery_target_candidates")
            .and_then(Value::as_array)
        else {
            continue;
        };
        out.extend(
            targets
                .iter()
                .filter_map(|target_value| target_value.get("target"))
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }
    out
}

fn current_workspace_scalar_count_targets_from_boundary_observation_blocks(
    summary: &str,
) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(scope) = value.get("current_workspace_scope") else {
            continue;
        };
        if !current_workspace_scope_marks_scalar_count(scope) {
            continue;
        }
        let Some(target) = scope
            .get("target")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        out.push(target.to_string());
    }
    out
}

fn current_workspace_scope_marks_scalar_count(scope: &Value) -> bool {
    const SCALAR_COUNT_MARKER: &str = "scalar_count";
    ["task_shape", "contract_marker", "output_contract_marker"]
        .into_iter()
        .filter_map(|key| scope.get(key).and_then(Value::as_str))
        .map(str::trim)
        .any(|value| value == SCALAR_COUNT_MARKER)
}

fn active_listing_bound_targets_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(targets) = value.get("active_bound_targets").and_then(Value::as_array) else {
            continue;
        };
        for target in targets {
            if !active_target_observation_has_listing_evidence(target) {
                continue;
            }
            let Some(target) = target
                .get("target")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            out.push(target.to_string());
        }
    }
    out
}

fn active_target_observation_has_listing_evidence(value: &Value) -> bool {
    value
        .get("op_kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "list")
        || value
            .get("ordered_entry_count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
        || value
            .get("observed_entry_count")
            .and_then(Value::as_u64)
            .is_some()
}
