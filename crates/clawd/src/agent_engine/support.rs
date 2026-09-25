use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path};
use toml::Value as TomlValue;
use tracing::{debug, info, warn};

use crate::task_lifecycle::{
    CheckpointBudgetCounters, ResumeEntrypoint, TaskCheckpoint, TaskLifecycleState,
};
use crate::{repo, AgentAction, AppState, ClaimedTask};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LoopRecipeOverrides {
    pub(super) max_actions_per_turn: Option<usize>,
    pub(super) repeat_action_limit: Option<usize>,
    pub(super) max_repairs: Option<usize>,
    pub(super) run_cmd_timeout_seconds: Option<u64>,
    pub(super) run_cmd_validation_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoopBudgetProfile {
    General,
    FastRead,
    GroundedSummary,
    MultiStepWorkspace,
    OpsClosedLoop,
}

impl LoopBudgetProfile {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::FastRead => "fast_read",
            Self::GroundedSummary => "grounded_summary",
            Self::MultiStepWorkspace => "multi_step_workspace",
            Self::OpsClosedLoop => "ops_closed_loop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistryIdempotencyGuardScope {
    #[cfg(test)]
    Off,
    All,
}

impl RegistryIdempotencyGuardScope {
    fn from_token(key: &'static str, token: &str) -> Self {
        if token.trim() != "all" {
            warn!(
                key,
                configured_scope = token,
                effective_scope = "all",
                reason_code = "legacy_guard_scope_normalized_to_all",
                "agent_loop_guard_final_scope_normalized"
            );
        }
        Self::All
    }

    fn enabled(self) -> bool {
        #[cfg(test)]
        if matches!(self, Self::Off) {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnswerVerifierRequiredEvidenceScope {
    #[cfg(test)]
    Off,
    All,
}

impl AnswerVerifierRequiredEvidenceScope {
    fn from_token(key: &'static str, token: &str) -> Self {
        if token.trim() != "all" {
            warn!(
                key,
                configured_scope = token,
                effective_scope = "all",
                reason_code = "legacy_guard_scope_normalized_to_all",
                "agent_loop_guard_final_scope_normalized"
            );
        }
        Self::All
    }

    fn enabled(self) -> bool {
        #[cfg(test)]
        if matches!(self, Self::Off) {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone)]
pub(super) struct AgentLoopGuardPolicy {
    pub(super) max_actions_per_turn: usize,
    pub(super) repeat_action_limit: usize,
    pub(super) answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope,
    pub(super) registry_idempotency_guard_scope: RegistryIdempotencyGuardScope,
    pub(super) fast_read: LoopRecipeOverrides,
    pub(super) grounded_summary: LoopRecipeOverrides,
    pub(super) multi_step_workspace: LoopRecipeOverrides,
    pub(super) ops_closed_loop: LoopRecipeOverrides,
}

impl AgentLoopGuardPolicy {
    pub(super) fn enabled_rollout_switches(&self) -> Vec<&'static str> {
        let mut switches = Vec::new();
        if self
            .effective_answer_verifier_required_evidence_scope()
            .enabled()
        {
            switches.push("answer_verifier_enforce_required_scope");
        }
        if self.effective_registry_idempotency_guard_scope().enabled() {
            switches.push("registry_idempotency_guard_scope");
        }
        switches
    }

    pub(super) fn effective_answer_verifier_required_evidence_scope(
        &self,
    ) -> AnswerVerifierRequiredEvidenceScope {
        self.answer_verifier_enforce_required_scope
    }

    pub(super) fn answer_verifier_required_evidence_enabled(&self) -> bool {
        self.effective_answer_verifier_required_evidence_scope()
            .enabled()
    }

    pub(super) fn effective_registry_idempotency_guard_scope(
        &self,
    ) -> RegistryIdempotencyGuardScope {
        self.registry_idempotency_guard_scope
    }

    pub(super) fn registry_idempotency_guard_enabled(&self) -> bool {
        self.effective_registry_idempotency_guard_scope().enabled()
    }

    pub(super) fn budget_profile_for_context(
        recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
        output_contract: Option<&crate::IntentOutputContract>,
    ) -> LoopBudgetProfile {
        if matches!(
            recipe.kind,
            crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
        ) {
            return LoopBudgetProfile::OpsClosedLoop;
        }

        let Some(output_contract) = output_contract else {
            return LoopBudgetProfile::General;
        };
        let operation = crate::evidence_policy::operation_for_output_contract(output_contract);
        let target_object =
            crate::evidence_policy::target_object_for_output_contract(output_contract);
        let required_evidence_fields =
            crate::evidence_policy::required_evidence_fields_for_output_contract(output_contract);
        let evidence_required = output_contract.requires_content_evidence
            || output_contract.delivery_required
            || !required_evidence_fields.is_empty();
        if output_contract.delivery_required {
            return LoopBudgetProfile::MultiStepWorkspace;
        }
        if matches!(
            target_object,
            crate::evidence_policy::EvidenceTargetObject::Directory
        ) && required_evidence_fields.len() >= 2
        {
            return LoopBudgetProfile::MultiStepWorkspace;
        }
        if required_evidence_fields.len() >= 2
            || (evidence_required
                && matches!(
                    operation,
                    crate::evidence_policy::EvidenceOperation::Run
                        | crate::evidence_policy::EvidenceOperation::List
                        | crate::evidence_policy::EvidenceOperation::Inspect
                ))
        {
            return LoopBudgetProfile::GroundedSummary;
        }

        LoopBudgetProfile::FastRead
    }

    fn overrides_for_profile(&self, profile: LoopBudgetProfile) -> LoopRecipeOverrides {
        match profile {
            LoopBudgetProfile::FastRead => self.fast_read,
            LoopBudgetProfile::GroundedSummary => self.grounded_summary,
            LoopBudgetProfile::MultiStepWorkspace => self.multi_step_workspace,
            LoopBudgetProfile::OpsClosedLoop => self.ops_closed_loop,
            LoopBudgetProfile::General => LoopRecipeOverrides::default(),
        }
    }

    pub(super) fn adjusted_for_context(
        &self,
        recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
        output_contract: Option<&crate::IntentOutputContract>,
    ) -> Self {
        let profile = Self::budget_profile_for_context(recipe, output_contract);
        self.adjusted_for_loop_budget_profile(profile)
    }

    pub(super) fn adjusted_for_task_budget_profile(
        &self,
        profile: crate::task_budget_contract::TaskBudgetProfile,
    ) -> Self {
        let profile = match profile {
            crate::task_budget_contract::TaskBudgetProfile::General => LoopBudgetProfile::General,
            crate::task_budget_contract::TaskBudgetProfile::FastRead => LoopBudgetProfile::FastRead,
            crate::task_budget_contract::TaskBudgetProfile::GroundedSummary => {
                LoopBudgetProfile::GroundedSummary
            }
            crate::task_budget_contract::TaskBudgetProfile::MultiStepWorkspace => {
                LoopBudgetProfile::MultiStepWorkspace
            }
            crate::task_budget_contract::TaskBudgetProfile::OpsClosedLoop => {
                LoopBudgetProfile::OpsClosedLoop
            }
        };
        self.adjusted_for_loop_budget_profile(profile)
    }

    fn adjusted_for_loop_budget_profile(&self, profile: LoopBudgetProfile) -> Self {
        let overrides = self.overrides_for_profile(profile);
        let mut policy = self.clone();
        if let Some(max_actions_per_turn) = overrides.max_actions_per_turn {
            policy.max_actions_per_turn = max_actions_per_turn;
        }
        if let Some(repeat_action_limit) = overrides.repeat_action_limit {
            policy.repeat_action_limit = repeat_action_limit;
        }
        policy
    }

    pub(super) fn apply_recipe_runtime_overrides(
        &self,
        recipe: &mut crate::execution_recipe::ExecutionRecipeRuntimeState,
    ) {
        let overrides = self.overrides_for_profile(Self::budget_profile_for_context(*recipe, None));
        if let Some(max_repairs) = overrides.max_repairs {
            recipe.max_repairs = max_repairs;
        }
    }

    pub(super) fn run_cmd_timeout_override(
        &self,
        recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
        action_effect: crate::execution_recipe::ActionEffect,
    ) -> Option<u64> {
        let overrides = self.overrides_for_profile(Self::budget_profile_for_context(recipe, None));
        if action_effect.validates {
            overrides
                .run_cmd_validation_timeout_seconds
                .or(overrides.run_cmd_timeout_seconds)
        } else {
            overrides.run_cmd_timeout_seconds
        }
    }
}

fn parse_usize_from_toml(root: &TomlValue, path: &[&str], fallback: usize) -> usize {
    let mut cursor = root;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return fallback;
        };
        cursor = next;
    }
    cursor
        .as_integer()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v >= 1)
        .unwrap_or(fallback)
}

fn parse_optional_usize_from_toml(root: &TomlValue, path: &[&str]) -> Option<usize> {
    let mut cursor = root;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return None;
        };
        cursor = next;
    }
    cursor
        .as_integer()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v >= 1)
}

fn parse_optional_u64_from_toml(root: &TomlValue, path: &[&str]) -> Option<u64> {
    let mut cursor = root;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return None;
        };
        cursor = next;
    }
    cursor
        .as_integer()
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v >= 1)
}

fn parse_migrated_action_limit(
    root: &TomlValue,
    prefix: &[&str],
    default: Option<usize>,
) -> Option<usize> {
    let mut current_path = prefix.to_vec();
    current_path.push("max_actions_per_turn");
    if let Some(value) = parse_optional_usize_from_toml(root, &current_path) {
        return Some(value);
    }
    let mut legacy_path = prefix.to_vec();
    legacy_path.push("max_steps");
    let legacy = parse_optional_usize_from_toml(root, &legacy_path);
    if legacy.is_some() {
        warn!(
            legacy_key = "max_steps",
            replacement_key = "max_actions_per_turn",
            reason_code = "agent_loop_action_limit_key_migrated",
            "agent_loop_guard_legacy_key_loaded"
        );
    }
    legacy.or(default)
}

fn parse_answer_verifier_required_evidence_scope(
    root: &TomlValue,
) -> AnswerVerifierRequiredEvidenceScope {
    let mut cursor = root;
    for key in [
        "agent",
        "loop_guard",
        "answer_verifier_enforce_required_scope",
    ] {
        let Some(next) = cursor.get(key) else {
            return AnswerVerifierRequiredEvidenceScope::All;
        };
        cursor = next;
    }
    AnswerVerifierRequiredEvidenceScope::from_token(
        "answer_verifier_enforce_required_scope",
        cursor.as_str().unwrap_or(""),
    )
}

fn parse_registry_idempotency_guard_scope(root: &TomlValue) -> RegistryIdempotencyGuardScope {
    let mut cursor = root;
    for key in ["agent", "loop_guard", "registry_idempotency_guard_scope"] {
        let Some(next) = cursor.get(key) else {
            return RegistryIdempotencyGuardScope::All;
        };
        cursor = next;
    }
    RegistryIdempotencyGuardScope::from_token(
        "registry_idempotency_guard_scope",
        cursor.as_str().unwrap_or(""),
    )
}

fn parse_loop_recipe_overrides(root: &TomlValue, path: &[&str]) -> LoopRecipeOverrides {
    let mut repeat_action_limit_path = path.to_vec();
    repeat_action_limit_path.push("repeat_action_limit");
    let mut max_repairs_path = path.to_vec();
    max_repairs_path.push("max_repairs");
    let mut run_cmd_timeout_path = path.to_vec();
    run_cmd_timeout_path.push("run_cmd_timeout_seconds");
    let mut run_cmd_validation_timeout_path = path.to_vec();
    run_cmd_validation_timeout_path.push("run_cmd_validation_timeout_seconds");

    LoopRecipeOverrides {
        max_actions_per_turn: parse_migrated_action_limit(root, path, None),
        repeat_action_limit: parse_optional_usize_from_toml(root, &repeat_action_limit_path),
        max_repairs: parse_optional_usize_from_toml(root, &max_repairs_path),
        run_cmd_timeout_seconds: parse_optional_u64_from_toml(root, &run_cmd_timeout_path),
        run_cmd_validation_timeout_seconds: parse_optional_u64_from_toml(
            root,
            &run_cmd_validation_timeout_path,
        ),
    }
}

pub(super) fn load_agent_loop_guard_policy(state: &AppState) -> AgentLoopGuardPolicy {
    let path = state
        .skill_rt
        .workspace_root
        .join("configs/agent_guard.toml");
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<TomlValue>(&raw).ok())
        .unwrap_or(TomlValue::Table(Default::default()));
    let answer_verifier_enforce_required_scope =
        parse_answer_verifier_required_evidence_scope(&parsed);
    let registry_idempotency_guard_scope = parse_registry_idempotency_guard_scope(&parsed);
    let policy = AgentLoopGuardPolicy {
        max_actions_per_turn: parse_migrated_action_limit(
            &parsed,
            &["agent", "loop_guard"],
            Some(crate::AGENT_MAX_ACTIONS_PER_TURN),
        )
        .unwrap_or(crate::AGENT_MAX_ACTIONS_PER_TURN),
        repeat_action_limit: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "repeat_action_limit"],
            4,
        ),
        answer_verifier_enforce_required_scope,
        registry_idempotency_guard_scope,
        fast_read: parse_loop_recipe_overrides(
            &parsed,
            &["agent", "loop_guard", "budget_profiles", "fast_read"],
        ),
        grounded_summary: parse_loop_recipe_overrides(
            &parsed,
            &["agent", "loop_guard", "budget_profiles", "grounded_summary"],
        ),
        multi_step_workspace: parse_loop_recipe_overrides(
            &parsed,
            &[
                "agent",
                "loop_guard",
                "budget_profiles",
                "multi_step_workspace",
            ],
        ),
        ops_closed_loop: parse_loop_recipe_overrides(
            &parsed,
            &["agent", "loop_guard", "ops_closed_loop"],
        ),
    };
    let enabled_rollout_switches = policy.enabled_rollout_switches();
    if !enabled_rollout_switches.is_empty() {
        info!(
            rollout_switches = enabled_rollout_switches.join(","),
            "agent_loop_guard_rollout_switches_enabled"
        );
    }
    policy
}

/// Publish progress hints only. Used for "in progress" UI. Must not contain full raw tool/skill output.
fn publish_progress(state: &AppState, task: &ClaimedTask, progress_messages: &[String]) {
    if progress_messages.is_empty() {
        return;
    }
    let payload = json!({
        "progress_messages": progress_messages,
        "task_lifecycle": {
            "schema_version": 1,
            "state": "running",
            "source": "agent_progress",
            "can_poll": true,
            "can_cancel": true,
            "last_heartbeat_ts": crate::now_ts_u64() as i64,
        },
    });
    if let Err(err) = repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &payload.to_string(),
    ) {
        warn!(
            "run_agent_with_tools: task_id={} publish progress failed: {}",
            task.task_id, err
        );
    } else {
        debug!(
            "progress published task_id={} count={} last={}",
            task.task_id,
            progress_messages.len(),
            crate::truncate_for_log(progress_messages.last().map(|s| s.as_str()).unwrap_or(""))
        );
    }
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn saturating_u32_from_u64(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn loop_tool_elapsed_ms(loop_state: &super::LoopState) -> u64 {
    loop_state
        .executed_step_results
        .iter()
        .map(|step| {
            step.finished_at
                .saturating_sub(step.started_at)
                .saturating_mul(1000)
        })
        .fold(0u64, u64::saturating_add)
}

pub(super) fn checkpoint_budget_counters(
    loop_state: &super::LoopState,
    llm_calls: u64,
    llm_elapsed_ms: u64,
) -> CheckpointBudgetCounters {
    let tool_elapsed_ms = loop_tool_elapsed_ms(loop_state);
    CheckpointBudgetCounters {
        round: saturating_u32(loop_state.round_no),
        step: saturating_u32(loop_state.total_steps_executed),
        llm_calls: saturating_u32_from_u64(llm_calls),
        tool_calls: saturating_u32(loop_state.tool_calls_total),
        elapsed_ms: llm_elapsed_ms.saturating_add(tool_elapsed_ms),
        llm_elapsed_ms,
        tool_elapsed_ms,
    }
}

fn agent_loop_checkpoint_id(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    reason: &str,
) -> String {
    format!(
        "agent-loop:{}:round-{}:step-{}:{}",
        task.task_id, loop_state.round_no, loop_state.total_steps_executed, reason
    )
}

fn checkpoint_step_observations(loop_state: &super::LoopState) -> Vec<Value> {
    loop_state
        .executed_step_results
        .iter()
        .map(|step| {
            json!({
                "step_id": step.step_id,
                "skill": step.skill,
                "status": step.status.as_str(),
                "has_output": step.output.as_deref().is_some_and(|value| !value.trim().is_empty()),
                "has_error": step.error.as_deref().is_some_and(|value| !value.trim().is_empty()),
            })
        })
        .collect()
}

fn completed_side_effect_refs(loop_state: &super::LoopState) -> Vec<String> {
    let mut refs = loop_state
        .successful_action_fingerprints
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    refs.sort();
    refs
}

fn push_changed_file_artifact_ref(refs: &mut Vec<String>, path: Option<&str>) {
    let Some(path) = path.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    refs.push(format!("changed_file:{path}"));
}

fn checkpoint_artifact_refs(loop_state: &super::LoopState) -> Vec<String> {
    let mut refs = Vec::new();
    push_changed_file_artifact_ref(&mut refs, loop_state.last_written_file_path.as_deref());
    push_changed_file_artifact_ref(
        &mut refs,
        loop_state
            .output_vars
            .get("last_written_file_path")
            .map(String::as_str),
    );
    push_changed_file_artifact_ref(
        &mut refs,
        loop_state
            .output_vars
            .get("last_file_path")
            .map(String::as_str),
    );
    for path in loop_state.written_file_aliases.values() {
        push_changed_file_artifact_ref(&mut refs, Some(path));
    }
    refs.sort();
    refs.dedup();
    refs
}

pub(crate) fn persist_agent_loop_clarification_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut super::LoopState,
    text: &str,
    machine_fields: Value,
) -> Result<crate::repo::conversation_reply_items::PersistConversationReplyOutcome, String> {
    if !machine_fields.is_object() {
        return Err("conversation_clarification_machine_fields_invalid".to_string());
    }
    if let Some(slice) = loop_state.task_budget_slice.as_mut() {
        slice.set_decision(crate::task_budget_contract::BudgetDecision::NeedsUser);
    }
    let reply_id = crate::repo::conversation_reply_items::nonterminal_reply_id(
        &task.task_id,
        "clarification",
        "accepted",
        text,
        loop_state.conversation_input_revision,
        loop_state.conversation_execution_epoch,
    );
    let resume_reason = "structured_clarification_required";
    let now_ts = crate::now_ts_u64() as i64;
    let budget = checkpoint_budget_counters(
        loop_state,
        state.task_llm_call_count(&task.task_id),
        state.task_llm_elapsed_ms(&task.task_id),
    );
    let checkpoint_id = agent_loop_checkpoint_id(task, loop_state, resume_reason);
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({
            "schema_version": 1,
            "source": "agent_clarification",
            "task_id": task.task_id,
            "resume_reason": resume_reason,
            "reply_id": reply_id,
            "clarification": machine_fields,
            "agent_loop_resume_state": checkpoint_resume_state(
                loop_state,
                super::checkpoint_resume_state::AgentCheckpointStage::Planning,
            ),
            "task_budget_slice": loop_state
                .task_budget_slice
                .as_ref()
                .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
        }),
        last_successful_round: (loop_state.round_no > 0)
            .then_some(saturating_u32(loop_state.round_no)),
        last_successful_step: loop_state
            .executed_step_results
            .iter()
            .rev()
            .find(|step| step.is_ok())
            .map(|step| step.step_id.clone()),
        pending_action: None,
        observations: checkpoint_step_observations(loop_state),
        capability_results: loop_state.capability_results.clone(),
        evidence_refs: loop_state
            .executed_step_results
            .iter()
            .filter(|step| step.is_ok())
            .map(|step| step.step_id.clone())
            .collect(),
        artifact_refs: checkpoint_artifact_refs(loop_state),
        completed_side_effect_refs: completed_side_effect_refs(loop_state),
        budget: budget.clone(),
        attempt_ledger: super::attempt_ledger::build_attempt_ledger_snapshot(loop_state),
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::AwaitUserInput,
    };
    let lifecycle = json!({
        "schema_version": 1,
        "state": TaskLifecycleState::NeedsUser,
        "source": "agent_clarification",
        "resume_reason": resume_reason,
        "checkpoint_id": checkpoint_id,
        "reply_id": reply_id,
        "can_poll": true,
        "can_cancel": true,
        "last_heartbeat_ts": now_ts,
        "budget": budget,
    });
    let payload = json!({
        "progress_messages": loop_state.progress_messages,
        "task_lifecycle": lifecycle,
        "task_checkpoint": checkpoint.to_machine_json(),
    });
    let persisted =
        crate::repo::conversation_reply_items::persist_clarification_reply_with_checkpoint(
            state,
            task,
            text,
            loop_state.conversation_input_revision,
            loop_state.conversation_execution_epoch,
            &payload,
        )
        .map_err(|error| format!("conversation_clarification_checkpoint_failed:{error}"))?;
    loop_state.task_lifecycle = payload.get("task_lifecycle").cloned();
    loop_state.task_checkpoint = payload.get("task_checkpoint").cloned();
    loop_state.output_vars.insert(
        "agent_loop.resume_reason".to_string(),
        resume_reason.to_string(),
    );
    Ok(persisted)
}

fn checkpoint_resume_message_key(resume_reason: &str) -> Option<&'static str> {
    match resume_reason {
        "task_budget_slice_exhausted" => Some("clawd.task.task_budget_slice_exhausted"),
        "user_pause_requested" => Some("clawd.task.pause_requested"),
        _ => None,
    }
}

fn context_compaction_checkpoint_trigger_json(resume_reason: &str) -> Value {
    json!({
        "schema_version": 1,
        "trigger_kind": "before_background_checkpoint",
        "source": "agent_loop_checkpoint",
        "resume_reason": resume_reason,
    })
}

pub(super) fn attach_task_llm_metrics_checkpoint(
    state: &AppState,
    task_id: &str,
    payload: &mut Value,
) {
    let Some(boundary) = payload
        .pointer_mut("/task_checkpoint/boundary_context")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    boundary.insert(
        "task_llm_metrics".to_string(),
        state.task_llm_metrics_checkpoint_json(task_id),
    );
}

fn checkpoint_resume_state(
    loop_state: &super::LoopState,
    stage: super::checkpoint_resume_state::AgentCheckpointStage,
) -> Value {
    super::checkpoint_resume_state::build_checkpoint_resume_state(loop_state, stage)
}

pub(super) fn refresh_agent_loop_checkpoint_snapshot(loop_state: &mut super::LoopState) {
    let resume_state = checkpoint_resume_state(
        loop_state,
        super::checkpoint_resume_state::AgentCheckpointStage::Planning,
    );
    let observations = checkpoint_step_observations(loop_state);
    let capability_results = loop_state.capability_results.clone();
    let evidence_refs = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok())
        .map(|step| step.step_id.clone())
        .collect::<Vec<_>>();
    let last_successful_step = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok())
        .map(|step| step.step_id.clone());
    let artifact_refs = checkpoint_artifact_refs(loop_state);
    let completed_side_effect_refs = completed_side_effect_refs(loop_state);
    let attempt_ledger = super::attempt_ledger::build_attempt_ledger_snapshot(loop_state);
    let Some(checkpoint) = loop_state
        .task_checkpoint
        .as_mut()
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let Some(boundary) = checkpoint
        .get_mut("boundary_context")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    boundary.insert("agent_loop_resume_state".to_string(), resume_state);
    checkpoint.insert("observations".to_string(), json!(observations));
    checkpoint.insert("capability_results".to_string(), json!(capability_results));
    checkpoint.insert("evidence_refs".to_string(), json!(evidence_refs));
    checkpoint.insert("artifact_refs".to_string(), json!(artifact_refs));
    checkpoint.insert(
        "completed_side_effect_refs".to_string(),
        json!(completed_side_effect_refs),
    );
    checkpoint.insert("attempt_ledger".to_string(), json!(attempt_ledger));
    checkpoint.insert(
        "last_successful_round".to_string(),
        json!((loop_state.round_no > 0).then_some(saturating_u32(loop_state.round_no))),
    );
    checkpoint.insert(
        "last_successful_step".to_string(),
        json!(last_successful_step),
    );
}

#[cfg(test)]
pub(super) fn build_agent_loop_checkpoint_progress_payload(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    resume_reason: &str,
    now_ts: i64,
    next_check_after: i64,
) -> Value {
    build_agent_loop_checkpoint_progress_payload_with_budget(
        task,
        loop_state,
        resume_reason,
        now_ts,
        next_check_after,
        checkpoint_budget_counters(loop_state, 0, 0),
    )
}

pub(super) fn build_agent_loop_checkpoint_progress_payload_with_budget(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    resume_reason: &str,
    now_ts: i64,
    next_check_after: i64,
    budget: CheckpointBudgetCounters,
) -> Value {
    let checkpoint_id = agent_loop_checkpoint_id(task, loop_state, resume_reason);
    let last_successful_step = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok())
        .map(|step| step.step_id.clone());
    let evidence_refs = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok())
        .map(|step| step.step_id.clone())
        .collect::<Vec<_>>();
    let message_key = checkpoint_resume_message_key(resume_reason);
    let mut boundary_context = json!({
        "schema_version": 1,
        "source": "agent_loop_soft_budget",
        "task_id": task.task_id,
        "resume_reason": resume_reason,
        "task_budget_slice": loop_state
            .task_budget_slice
            .as_ref()
            .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
        "context_compaction_trigger": context_compaction_checkpoint_trigger_json(resume_reason),
        "agent_loop_resume_state": checkpoint_resume_state(
            loop_state,
            super::checkpoint_resume_state::AgentCheckpointStage::Planning,
        ),
    });
    if let (Some(obj), Some(message_key)) = (boundary_context.as_object_mut(), message_key) {
        obj.insert("message_key".to_string(), json!(message_key));
    }
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context,
        last_successful_round: (loop_state.round_no > 0)
            .then_some(saturating_u32(loop_state.round_no)),
        last_successful_step,
        pending_action: None,
        observations: checkpoint_step_observations(loop_state),
        capability_results: loop_state.capability_results.clone(),
        evidence_refs,
        artifact_refs: checkpoint_artifact_refs(loop_state),
        completed_side_effect_refs: completed_side_effect_refs(loop_state),
        budget: budget.clone(),
        attempt_ledger: super::attempt_ledger::build_attempt_ledger_snapshot(loop_state),
        pending_async_job: None,
        repair_signal: loop_state.last_stop_signal.as_ref().map(|signal| {
            crate::repair_signal::RepairSignal::from_checkpoint_resume_parts(
                &checkpoint_id,
                ResumeEntrypoint::NextPlannerRound,
                signal,
            )
            .to_json()
        }),
        resume_entrypoint: ResumeEntrypoint::NextPlannerRound,
    };

    let mut lifecycle = json!({
        "schema_version": 1,
        "state": TaskLifecycleState::Waiting,
        "source": "agent_loop_soft_budget",
        "resume_reason": resume_reason,
        "next_check_after": next_check_after.max(now_ts + 1),
        "checkpoint_id": checkpoint_id,
        "can_poll": true,
        "can_cancel": true,
        "last_heartbeat_ts": now_ts,
        "budget": budget,
        "context_compaction_trigger": context_compaction_checkpoint_trigger_json(resume_reason),
    });
    if let (Some(obj), Some(message_key)) = (lifecycle.as_object_mut(), message_key) {
        obj.insert("message_key".to_string(), json!(message_key));
    }

    json!({
        "progress_messages": loop_state.progress_messages,
        "task_lifecycle": lifecycle,
        "task_checkpoint": checkpoint.to_machine_json(),
        "task_budget_slice": loop_state
            .task_budget_slice
            .as_ref()
            .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
    })
}

#[cfg(test)]
pub(super) fn build_agent_loop_recovery_snapshot_payload(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    now_ts: i64,
) -> Value {
    build_agent_loop_recovery_snapshot_payload_with_budget(
        task,
        loop_state,
        now_ts,
        checkpoint_budget_counters(loop_state, 0, 0),
    )
}

fn build_agent_loop_recovery_snapshot_payload_with_budget(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    now_ts: i64,
    budget: CheckpointBudgetCounters,
) -> Value {
    let mut payload = build_agent_loop_checkpoint_progress_payload_with_budget(
        task,
        loop_state,
        "durable_action_boundary",
        now_ts,
        now_ts.saturating_add(1),
        budget,
    );
    if let Some(lifecycle) = payload
        .get_mut("task_lifecycle")
        .and_then(Value::as_object_mut)
    {
        lifecycle.insert("state".to_string(), json!(TaskLifecycleState::Running));
        lifecycle.insert("source".to_string(), json!("agent_loop_recovery_snapshot"));
        lifecycle.insert("recoverable_after_lease_loss".to_string(), json!(true));
        lifecycle.remove("next_check_after");
        lifecycle.remove("message_key");
    }
    if let Some(boundary) = payload
        .pointer_mut("/task_checkpoint/boundary_context")
        .and_then(Value::as_object_mut)
    {
        boundary.insert("source".to_string(), json!("agent_loop_recovery_snapshot"));
    }
    payload
}

pub(super) fn persist_agent_loop_recovery_snapshot(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &super::LoopState,
) {
    if loop_state.executed_step_results.is_empty()
        || loop_state
            .task_lifecycle
            .as_ref()
            .and_then(|lifecycle| lifecycle.get("state"))
            .and_then(Value::as_str)
            .is_some_and(|state| matches!(state, "waiting" | "background" | "needs_user"))
    {
        return;
    }
    let now_ts = crate::now_ts_u64() as i64;
    let budget = checkpoint_budget_counters(
        loop_state,
        state.task_llm_call_count(&task.task_id),
        state.task_llm_elapsed_ms(&task.task_id),
    );
    let mut payload =
        build_agent_loop_recovery_snapshot_payload_with_budget(task, loop_state, now_ts, budget);
    attach_task_llm_metrics_checkpoint(state, &task.task_id, &mut payload);
    if let Err(error) = repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &payload.to_string(),
    ) {
        warn!(
            "agent loop recovery snapshot persist failed task_id={} error={}",
            task.task_id, error
        );
    } else {
        debug!(
            "agent loop recovery snapshot persisted task_id={} round={} step={}",
            task.task_id, loop_state.round_no, loop_state.total_steps_executed
        );
    }
}

fn action_args_keys(args: &Value) -> Vec<String> {
    let Some(obj) = args.as_object() else {
        return Vec::new();
    };
    let mut keys = obj.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

#[cfg(test)]
pub(super) fn build_agent_loop_user_input_checkpoint_progress_payload(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    resume_reason: &str,
    now_ts: i64,
    tool_or_skill: &str,
    action_ref: &str,
    args: &Value,
) -> Value {
    build_agent_loop_user_input_checkpoint_progress_payload_with_budget(
        task,
        loop_state,
        resume_reason,
        now_ts,
        tool_or_skill,
        action_ref,
        args,
        checkpoint_budget_counters(loop_state, 0, 0),
    )
}

fn build_agent_loop_user_input_checkpoint_progress_payload_with_budget(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    resume_reason: &str,
    now_ts: i64,
    tool_or_skill: &str,
    action_ref: &str,
    args: &Value,
    budget: CheckpointBudgetCounters,
) -> Value {
    let checkpoint_id = agent_loop_checkpoint_id(task, loop_state, resume_reason);
    let policy_decision = crate::policy_decision::PolicyDecision::RequireConfirmation.as_token();
    let pending_action = json!({
        "schema_version": 1,
        "kind": "agent_hook_pre_tool_use",
        "tool_or_skill": tool_or_skill,
        "action_ref": action_ref,
        "args_keys": action_args_keys(args),
        "resume_expected": "user_followup",
    });
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({
            "schema_version": 1,
            "source": "agent_hooks",
            "stage": "pre_tool_use",
            "decision": policy_decision,
            "task_id": task.task_id,
            "resume_reason": resume_reason,
            "tool_or_skill": tool_or_skill,
            "action_ref": action_ref,
            "message_key": "clawd.agent_hook.confirmation_required",
            "agent_loop_resume_state": checkpoint_resume_state(
                loop_state,
                super::checkpoint_resume_state::AgentCheckpointStage::ToolExecution,
            ),
            "task_budget_slice": loop_state
                .task_budget_slice
                .as_ref()
                .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
        }),
        last_successful_round: (loop_state.round_no > 0)
            .then_some(saturating_u32(loop_state.round_no)),
        last_successful_step: loop_state
            .executed_step_results
            .iter()
            .rev()
            .find(|step| step.is_ok())
            .map(|step| step.step_id.clone()),
        pending_action: Some(pending_action),
        observations: checkpoint_step_observations(loop_state),
        capability_results: loop_state.capability_results.clone(),
        evidence_refs: loop_state
            .executed_step_results
            .iter()
            .filter(|step| step.is_ok())
            .map(|step| step.step_id.clone())
            .collect::<Vec<_>>(),
        artifact_refs: checkpoint_artifact_refs(loop_state),
        completed_side_effect_refs: completed_side_effect_refs(loop_state),
        budget: budget.clone(),
        attempt_ledger: super::attempt_ledger::build_attempt_ledger_snapshot(loop_state),
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::AwaitUserInput,
    };

    json!({
        "progress_messages": loop_state.progress_messages,
        "task_lifecycle": {
            "schema_version": 1,
            "state": TaskLifecycleState::NeedsUser,
            "source": "agent_hooks",
            "resume_reason": resume_reason,
            "checkpoint_id": checkpoint_id,
            "can_poll": true,
            "can_cancel": true,
            "last_heartbeat_ts": now_ts,
            "message_key": "clawd.agent_hook.confirmation_required",
            "stage": "pre_tool_use",
            "decision": policy_decision,
            "tool_or_skill": tool_or_skill,
            "action_ref": action_ref,
            "budget": budget,
            "task_budget_slice": loop_state
                .task_budget_slice
                .as_ref()
                .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
        },
        "task_checkpoint": checkpoint.to_machine_json(),
    })
}

pub(super) fn publish_agent_loop_checkpoint_progress(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut super::LoopState,
    resume_reason: &str,
) {
    let now_ts = crate::now_ts_u64() as i64;
    let budget = checkpoint_budget_counters(
        loop_state,
        state.task_llm_call_count(&task.task_id),
        state.task_llm_elapsed_ms(&task.task_id),
    );
    let next_check_after = now_ts.saturating_add(60);
    let mut payload = build_agent_loop_checkpoint_progress_payload_with_budget(
        task,
        loop_state,
        resume_reason,
        now_ts,
        next_check_after,
        budget,
    );
    persist_agent_loop_checkpoint_progress_payload(
        state,
        task,
        loop_state,
        resume_reason,
        &mut payload,
    );
}

pub(super) fn publish_agent_loop_pause_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut super::LoopState,
    resume_after: Option<i64>,
) {
    let now_ts = crate::now_ts_u64() as i64;
    let budget = checkpoint_budget_counters(
        loop_state,
        state.task_llm_call_count(&task.task_id),
        state.task_llm_elapsed_ms(&task.task_id),
    );
    let mut payload = build_agent_loop_checkpoint_progress_payload_with_budget(
        task,
        loop_state,
        "user_pause_requested",
        now_ts,
        resume_after.unwrap_or_else(|| now_ts.saturating_add(1)),
        budget,
    );
    if let Some(lifecycle) = payload
        .get_mut("task_lifecycle")
        .and_then(Value::as_object_mut)
    {
        lifecycle.insert("source".to_string(), json!("task_control"));
        if let Some(resume_after) = resume_after {
            lifecycle.insert("resume_policy".to_string(), json!("scheduled"));
            lifecycle.insert("resume_after".to_string(), json!(resume_after));
        } else {
            lifecycle.insert("state".to_string(), json!(TaskLifecycleState::NeedsUser));
            lifecycle.insert("resume_policy".to_string(), json!("manual"));
            lifecycle.insert("manual_resume_required".to_string(), json!(true));
            lifecycle.insert("resume_due".to_string(), json!(false));
            lifecycle.insert("resume_wait_seconds".to_string(), json!(0));
            lifecycle.remove("next_check_after");
        }
    }
    if let Some(boundary) = payload
        .pointer_mut("/task_checkpoint/boundary_context")
        .and_then(Value::as_object_mut)
    {
        boundary.insert("source".to_string(), json!("task_control"));
        boundary.insert(
            "resume_policy".to_string(),
            json!(if resume_after.is_some() {
                "scheduled"
            } else {
                "manual"
            }),
        );
        if let Some(resume_after) = resume_after {
            boundary.insert("resume_after".to_string(), json!(resume_after));
        }
    }
    persist_agent_loop_checkpoint_progress_payload(
        state,
        task,
        loop_state,
        "user_pause_requested",
        &mut payload,
    );
}

fn persist_agent_loop_checkpoint_progress_payload(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut super::LoopState,
    resume_reason: &str,
    payload: &mut Value,
) {
    attach_task_llm_metrics_checkpoint(state, &task.task_id, payload);
    if let Some(checkpoint_id) = payload
        .pointer("/task_lifecycle/checkpoint_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    {
        loop_state
            .output_vars
            .insert("agent_loop.checkpoint_id".to_string(), checkpoint_id);
    }
    loop_state.task_lifecycle = payload.get("task_lifecycle").cloned();
    loop_state.task_checkpoint = payload.get("task_checkpoint").cloned();
    loop_state.output_vars.insert(
        "agent_loop.resume_reason".to_string(),
        resume_reason.to_string(),
    );
    if let Err(err) = repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &payload.to_string(),
    ) {
        warn!(
            "run_agent_with_tools: task_id={} publish checkpoint progress failed: {}",
            task.task_id, err
        );
    } else {
        debug!(
            "checkpoint progress published task_id={} reason={}",
            task.task_id, resume_reason
        );
    }
}

pub(crate) fn publish_agent_loop_user_input_checkpoint_progress(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut super::LoopState,
    resume_reason: &str,
    tool_or_skill: &str,
    action_ref: &str,
    args: &Value,
    step_in_round: usize,
) -> Result<(), String> {
    if let Some(slice) = loop_state.task_budget_slice.as_mut() {
        slice.set_decision(crate::task_budget_contract::BudgetDecision::NeedsUser);
    }
    let now_ts = crate::now_ts_u64() as i64;
    let budget = checkpoint_budget_counters(
        loop_state,
        state.task_llm_call_count(&task.task_id),
        state.task_llm_elapsed_ms(&task.task_id),
    );
    let mut payload = build_agent_loop_user_input_checkpoint_progress_payload_with_budget(
        task,
        loop_state,
        resume_reason,
        now_ts,
        tool_or_skill,
        action_ref,
        args,
        budget,
    );
    attach_task_llm_metrics_checkpoint(state, &task.task_id, &mut payload);
    let checkpoint_id = payload
        .pointer("/task_lifecycle/checkpoint_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "checkpoint_action_checkpoint_id_missing".to_string())?;
    let output_contract = loop_state
        .output_contract
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| "checkpoint_action_output_contract_serialize_failed".to_string())?;
    let continuation_actions = checkpoint_continuation_actions(loop_state, step_in_round)?;
    let execution_binding =
        crate::skills::checkpoint_skill_execution_binding(state, tool_or_skill)?;
    let approval_binding = checkpoint_action_approval_binding(
        state,
        tool_or_skill,
        action_ref,
        args,
        loop_state.total_steps_executed,
        loop_state.output_contract.clone(),
        continuation_actions.as_ref(),
    )?;
    repo::upsert_task_checkpoint_action(
        &state.core.db,
        &task.task_id,
        &checkpoint_id,
        tool_or_skill,
        action_ref,
        args,
        output_contract.as_ref(),
        continuation_actions.as_ref(),
        Some(&execution_binding),
        approval_binding.as_ref(),
        loop_state.conversation_input_revision,
        loop_state.conversation_execution_epoch,
    )
    .map_err(|_| "checkpoint_action_persist_failed".to_string())?;
    loop_state.output_vars.insert(
        "agent_loop.checkpoint_id".to_string(),
        checkpoint_id.clone(),
    );
    loop_state.task_lifecycle = payload.get("task_lifecycle").cloned();
    loop_state.task_checkpoint = payload.get("task_checkpoint").cloned();
    loop_state.output_vars.insert(
        "agent_loop.resume_reason".to_string(),
        resume_reason.to_string(),
    );
    repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &payload.to_string(),
    )
    .map_err(|err| {
        warn!(
            "run_agent_with_tools: task_id={} publish user-input checkpoint failed: {}",
            task.task_id, err
        );
        "checkpoint_action_progress_publish_failed".to_string()
    })?;
    debug!(
        "user-input checkpoint progress published task_id={} checkpoint_id={} reason={} action_ref={}",
        task.task_id, checkpoint_id, resume_reason, action_ref
    );
    Ok(())
}

fn checkpoint_continuation_actions(
    loop_state: &super::LoopState,
    step_in_round: usize,
) -> Result<Option<Value>, String> {
    let actions = loop_state
        .active_verified_actions
        .get(step_in_round..)
        .unwrap_or_default();
    if actions.is_empty() {
        return Ok(None);
    }
    serde_json::to_value(actions)
        .map(Some)
        .map_err(|_| "checkpoint_continuation_actions_serialize_failed".to_string())
}

fn checkpoint_action_approval_binding(
    state: &AppState,
    tool_or_skill: &str,
    action_ref: &str,
    args: &Value,
    completed_step_count: usize,
    output_contract: Option<crate::IntentOutputContract>,
    continuation_actions: Option<&Value>,
) -> Result<Option<Value>, String> {
    let continuation_actions = continuation_actions
        .cloned()
        .map(serde_json::from_value::<Vec<crate::AgentAction>>)
        .transpose()
        .map_err(|_| "checkpoint_continuation_actions_invalid".to_string())?
        .unwrap_or_default();
    let plan = super::checkpoint_action_plan(
        tool_or_skill,
        action_ref,
        args.clone(),
        completed_step_count,
        output_contract,
        continuation_actions,
    );
    let Some(first_step_id) = plan.steps.first().map(|step| step.step_id.clone()) else {
        return Err("checkpoint_action_plan_empty".to_string());
    };
    Ok(
        crate::approval_grant::binding_for_confirmation_steps(state, &plan.steps, &[first_step_id])
            .map(|binding| {
                json!({
                    "schema_version": 1,
                    "action_fingerprint": binding.action_fingerprint,
                    "arguments_hash": binding.arguments_hash,
                    "action_count": binding.action_count,
                    "targets": binding.targets,
                })
            }),
    )
}

pub(super) fn publish_agent_loop_mutation_reconciliation_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut super::LoopState,
    action_ref: &str,
    fingerprint_hash: &str,
    ledger_status: &str,
) {
    if let Some(slice) = loop_state.task_budget_slice.as_mut() {
        slice.set_decision(crate::task_budget_contract::BudgetDecision::NeedsUser);
    }
    let now_ts = crate::now_ts_u64() as i64;
    let budget = checkpoint_budget_counters(
        loop_state,
        state.task_llm_call_count(&task.task_id),
        state.task_llm_elapsed_ms(&task.task_id),
    );
    let checkpoint_id =
        agent_loop_checkpoint_id(task, loop_state, "mutation_reconciliation_required");
    let pending_action = json!({
        "schema_version": 1,
        "kind": "mutation_reconciliation",
        "action_ref": action_ref,
        "fingerprint_hash": fingerprint_hash,
        "ledger_status": ledger_status,
        "resume_expected": "verified_effect_outcome",
    });
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({
            "schema_version": 1,
            "source": "task_mutation_ledger",
            "task_id": task.task_id,
            "reason_code": "mutation_outcome_unknown",
            "message_key": "clawd.task.mutation_outcome_unknown",
            "action_ref": action_ref,
            "fingerprint_hash": fingerprint_hash,
            "ledger_status": ledger_status,
            "requires_reconciliation": true,
            "agent_loop_resume_state": checkpoint_resume_state(
                loop_state,
                super::checkpoint_resume_state::AgentCheckpointStage::ToolExecution,
            ),
            "task_budget_slice": loop_state
                .task_budget_slice
                .as_ref()
                .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
        }),
        last_successful_round: (loop_state.round_no > 0)
            .then_some(saturating_u32(loop_state.round_no)),
        last_successful_step: loop_state
            .executed_step_results
            .iter()
            .rev()
            .find(|step| step.is_ok())
            .map(|step| step.step_id.clone()),
        pending_action: Some(pending_action),
        observations: checkpoint_step_observations(loop_state),
        capability_results: loop_state.capability_results.clone(),
        evidence_refs: loop_state
            .executed_step_results
            .iter()
            .filter(|step| step.is_ok())
            .map(|step| step.step_id.clone())
            .collect(),
        artifact_refs: checkpoint_artifact_refs(loop_state),
        completed_side_effect_refs: completed_side_effect_refs(loop_state),
        budget: budget.clone(),
        attempt_ledger: super::attempt_ledger::build_attempt_ledger_snapshot(loop_state),
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::AwaitUserInput,
    };
    let lifecycle = json!({
        "schema_version": 1,
        "state": TaskLifecycleState::NeedsUser,
        "source": "task_mutation_ledger",
        "resume_reason": "mutation_reconciliation_required",
        "pause_reason_code": "mutation_outcome_unknown",
        "next_action": "verify_mutation_outcome",
        "checkpoint_id": checkpoint_id,
        "can_poll": false,
        "can_cancel": true,
        "last_heartbeat_ts": now_ts,
        "message_key": "clawd.task.mutation_outcome_unknown",
        "action_ref": action_ref,
        "fingerprint_hash": fingerprint_hash,
        "ledger_status": ledger_status,
        "requires_reconciliation": true,
        "budget": budget,
    });
    let payload = json!({
        "progress_messages": loop_state.progress_messages,
        "task_lifecycle": lifecycle,
        "task_checkpoint": checkpoint.to_machine_json(),
    });
    loop_state.task_lifecycle = payload.get("task_lifecycle").cloned();
    loop_state.task_checkpoint = payload.get("task_checkpoint").cloned();
    loop_state.output_vars.insert(
        "agent_loop.resume_reason".to_string(),
        "mutation_reconciliation_required".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.mutation_reconciliation_required".to_string(),
        "true".to_string(),
    );
    if let Err(error) = repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &payload.to_string(),
    ) {
        warn!(
            "run_agent_with_tools: task_id={} publish mutation reconciliation checkpoint failed: {}",
            task.task_id, error
        );
    } else {
        debug!(
            "mutation reconciliation checkpoint published task_id={} action_ref={}",
            task.task_id, action_ref
        );
    }
}

/// Max length for args summary in progress hint. Longer summaries are truncated with "...".
pub(super) const PROGRESS_ARGS_SUMMARY_MAX_LEN: usize = 160;

/// Keys allowed in progress hint args summary (fixed order). Any other key is omitted.
const PROGRESS_ARGS_WHITELIST: &[&str] = &[
    "action",
    "exchange",
    "symbol",
    "side",
    "order_type",
    "quote_qty_usd",
    "qty",
    "price",
    "stop_price",
    "time_in_force",
    "limit",
    "order_id",
    "client_order_id",
];

/// Keys that must never appear in progress hint (case-insensitive substring match).
const PROGRESS_ARGS_SENSITIVE: &[&str] = &[
    "api_key",
    "api_secret",
    "passphrase",
    "user_key",
    "authorization",
    "token",
    "credential",
    "secret",
    "password",
];

fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_lowercase();
    PROGRESS_ARGS_SENSITIVE
        .iter()
        .any(|s| k.contains(&s.to_lowercase()))
}

fn value_to_short_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.as_str().trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

/// Build a safe, whitelisted args summary for progress hint. No sensitive keys; truncated to max_len.
pub(crate) fn build_safe_skill_args_summary(args: &Value, max_len: usize) -> String {
    let obj = match args.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    let mut parts: Vec<String> = Vec::new();
    for &key in PROGRESS_ARGS_WHITELIST {
        if is_sensitive_key(key) {
            continue;
        }
        let Some(v) = obj.get(key) else { continue };
        let s = value_to_short_string(v);
        if s.is_empty() {
            continue;
        }
        let val_display = if s.len() > 40 {
            format!("{}...", &s[..37])
        } else {
            s
        };
        parts.push(format!("{key}={val_display}"));
    }
    let summary = parts.join(", ");
    if summary.len() <= max_len {
        summary
    } else {
        format!(
            "{}...",
            summary
                .chars()
                .take(max_len.saturating_sub(3))
                .collect::<String>()
        )
    }
}

/// Encode a progress hint for telegramd to render with its i18n. Format: "I18N:key:json_vars".
pub(crate) fn encode_progress_i18n(key: &str, vars: &[(&str, &str)]) -> String {
    let obj: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let vars_json = serde_json::to_string(&obj).unwrap_or_else(|_| "{}".to_string());
    format!("I18N:{}:{}", key, vars_json)
}

/// Append a short progress hint and publish. For "processing..." display only. Do not pass full raw output.
pub(super) fn append_progress_hint(
    state: &AppState,
    task: &ClaimedTask,
    progress_messages: &mut Vec<String>,
    hint: String,
) {
    progress_messages.push(hint);
    publish_progress(state, task, progress_messages);
}

fn collect_execution_recipe_progress_hints(loop_state: &mut super::LoopState) -> Vec<String> {
    let recipe = loop_state.execution_recipe;
    if !recipe.is_active() {
        return Vec::new();
    }
    let mut hints = Vec::new();

    if loop_state.last_recipe_progress_scope != Some(recipe.target_scope) {
        let mode_hint = match recipe.target_scope {
            crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace => Some(
                encode_progress_i18n("telegram.progress.ops_recipe_scope_external_mode", &[]),
            ),
            crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield => Some(
                encode_progress_i18n("telegram.progress.ops_recipe_scope_greenfield_mode", &[]),
            ),
            crate::execution_recipe::ExecutionRecipeTargetScope::Unknown
            | crate::execution_recipe::ExecutionRecipeTargetScope::System
            | crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => None,
        };
        loop_state.last_recipe_progress_scope = Some(recipe.target_scope);
        if let Some(hint) = mode_hint {
            hints.push(hint);
        }
    }

    if !loop_state.recipe_scope_ready_hint_sent {
        let ready_hint = match recipe.target_scope {
            crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace
                if recipe.saw_external_target =>
            {
                Some(encode_progress_i18n(
                    "telegram.progress.ops_recipe_scope_external_ready",
                    &[],
                ))
            }
            crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield
                if recipe.saw_greenfield_creation =>
            {
                Some(encode_progress_i18n(
                    "telegram.progress.ops_recipe_scope_greenfield_ready",
                    &[],
                ))
            }
            _ => None,
        };
        if let Some(hint) = ready_hint {
            loop_state.recipe_scope_ready_hint_sent = true;
            hints.push(hint);
        }
    }

    if loop_state.last_recipe_progress_phase != Some(recipe.phase) {
        let hint = match recipe.phase {
            crate::execution_recipe::ExecutionRecipePhase::Inspect => encode_progress_i18n(
                execution_recipe_phase_progress_key(
                    recipe.profile,
                    crate::execution_recipe::ExecutionRecipePhase::Inspect,
                ),
                &[],
            ),
            crate::execution_recipe::ExecutionRecipePhase::Apply => encode_progress_i18n(
                execution_recipe_phase_progress_key(
                    recipe.profile,
                    crate::execution_recipe::ExecutionRecipePhase::Apply,
                ),
                &[],
            ),
            crate::execution_recipe::ExecutionRecipePhase::Validate => encode_progress_i18n(
                execution_recipe_phase_progress_key(
                    recipe.profile,
                    crate::execution_recipe::ExecutionRecipePhase::Validate,
                ),
                &[],
            ),
            crate::execution_recipe::ExecutionRecipePhase::Repair => encode_progress_i18n(
                "telegram.progress.ops_recipe_repair",
                &[
                    ("attempt", &recipe.repair_count.to_string()),
                    ("max_repairs", &recipe.max_repairs.to_string()),
                ],
            ),
            crate::execution_recipe::ExecutionRecipePhase::Done => return hints,
        };
        loop_state.last_recipe_progress_phase = Some(recipe.phase);
        hints.push(hint);
    }

    hints
}

fn execution_recipe_phase_progress_key(
    profile: crate::execution_recipe::ExecutionRecipeProfile,
    phase: crate::execution_recipe::ExecutionRecipePhase,
) -> &'static str {
    match (profile, phase) {
        (
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            crate::execution_recipe::ExecutionRecipePhase::Inspect,
        ) => "telegram.progress.config_change_inspect",
        (
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            crate::execution_recipe::ExecutionRecipePhase::Apply,
        ) => "telegram.progress.config_change_apply",
        (
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            crate::execution_recipe::ExecutionRecipePhase::Validate,
        ) => "telegram.progress.config_change_validate",
        (
            crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            crate::execution_recipe::ExecutionRecipePhase::Inspect,
        ) => "telegram.progress.code_change_inspect",
        (
            crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            crate::execution_recipe::ExecutionRecipePhase::Apply,
        ) => "telegram.progress.code_change_apply",
        (
            crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            crate::execution_recipe::ExecutionRecipePhase::Validate,
        ) => "telegram.progress.code_change_validate",
        (
            crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring,
            crate::execution_recipe::ExecutionRecipePhase::Inspect,
        ) => "telegram.progress.skill_authoring_inspect",
        (
            crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring,
            crate::execution_recipe::ExecutionRecipePhase::Apply,
        ) => "telegram.progress.skill_authoring_apply",
        (
            crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring,
            crate::execution_recipe::ExecutionRecipePhase::Validate,
        ) => "telegram.progress.skill_authoring_validate",
        _ => match phase {
            crate::execution_recipe::ExecutionRecipePhase::Inspect => {
                "telegram.progress.ops_recipe_inspect"
            }
            crate::execution_recipe::ExecutionRecipePhase::Apply => {
                "telegram.progress.ops_recipe_apply"
            }
            crate::execution_recipe::ExecutionRecipePhase::Validate => {
                "telegram.progress.ops_recipe_validate"
            }
            crate::execution_recipe::ExecutionRecipePhase::Repair => {
                "telegram.progress.ops_recipe_repair"
            }
            crate::execution_recipe::ExecutionRecipePhase::Done => {
                "telegram.progress.reply_generated"
            }
        },
    }
}

pub(super) fn maybe_publish_execution_recipe_phase_hint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut super::LoopState,
) {
    for hint in collect_execution_recipe_progress_hints(loop_state) {
        append_progress_hint(state, task, &mut loop_state.progress_messages, hint);
    }
}

include!("support_action_identity.rs");
#[cfg(test)]
#[path = "support_tests.rs"]
mod tests;
include!("idempotency_fingerprint.rs");
