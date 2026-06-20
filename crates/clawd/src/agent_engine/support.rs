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
    pub(super) max_steps: Option<usize>,
    pub(super) max_rounds: Option<usize>,
    pub(super) max_tool_calls: Option<usize>,
    pub(super) repeat_action_limit: Option<usize>,
    pub(super) no_progress_limit: Option<usize>,
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
pub(super) enum SemanticRouteAuthority {
    Legacy,
    Shadow,
    AgentLoopCanary,
    AgentLoopDefault,
}

impl SemanticRouteAuthority {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Shadow => "shadow",
            Self::AgentLoopCanary => "agent_loop_canary",
            Self::AgentLoopDefault => "agent_loop_default",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        match token.trim() {
            "legacy" => Some(Self::Legacy),
            "shadow" => Some(Self::Shadow),
            "agent_loop_canary" => Some(Self::AgentLoopCanary),
            "agent_loop_default" => Some(Self::AgentLoopDefault),
            _ => None,
        }
    }

    fn records_agent_decides_attribution(self) -> bool {
        !matches!(self, Self::Legacy)
    }

    pub(super) fn uses_agent_loop_authority(self) -> bool {
        matches!(self, Self::AgentLoopCanary | Self::AgentLoopDefault)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistryIdempotencyGuardScope {
    Off,
    SelectedAgentLoop,
    All,
}

impl RegistryIdempotencyGuardScope {
    fn from_token(token: &str) -> Option<Self> {
        match token.trim() {
            "off" => Some(Self::Off),
            "selected_agent_loop" => Some(Self::SelectedAgentLoop),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnswerVerifierRequiredEvidenceScope {
    Off,
    SelectedAgentLoop,
    All,
}

impl AnswerVerifierRequiredEvidenceScope {
    fn from_token(token: &str) -> Option<Self> {
        match token.trim() {
            "off" => Some(Self::Off),
            "selected_agent_loop" => Some(Self::SelectedAgentLoop),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct AgentLoopGuardPolicy {
    pub(super) max_steps: usize,
    pub(super) max_rounds: usize,
    pub(super) max_tool_calls: usize,
    pub(super) recoverable_failure_extra_rounds: usize,
    pub(super) repeat_action_limit: usize,
    pub(super) no_progress_limit: usize,
    pub(super) multi_round_enabled: bool,
    pub(super) answer_verifier_retry_limit: usize,
    pub(super) answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope,
    pub(super) semantic_route_authority: SemanticRouteAuthority,
    pub(super) agent_loop_canary_bucket: String,
    pub(super) registry_idempotency_guard_scope: RegistryIdempotencyGuardScope,
    pub(super) structured_evidence_required_for_selected_contracts: bool,
    pub(super) fast_read: LoopRecipeOverrides,
    pub(super) grounded_summary: LoopRecipeOverrides,
    pub(super) multi_step_workspace: LoopRecipeOverrides,
    pub(super) ops_closed_loop: LoopRecipeOverrides,
}

impl AgentLoopGuardPolicy {
    pub(super) fn effective_semantic_route_authority(&self) -> SemanticRouteAuthority {
        self.semantic_route_authority
    }

    pub(super) fn records_agent_decides_attribution(&self) -> bool {
        self.effective_semantic_route_authority()
            .records_agent_decides_attribution()
    }

    pub(super) fn uses_agent_loop_semantic_authority(&self) -> bool {
        self.effective_semantic_route_authority()
            .uses_agent_loop_authority()
    }

    pub(super) fn selected_migration_class_for_eligible(
        &self,
        eligible_migration_class: &'static str,
    ) -> &'static str {
        if eligible_migration_class == "none" {
            return "none";
        }
        if self.effective_semantic_route_authority() == SemanticRouteAuthority::AgentLoopDefault {
            return eligible_migration_class;
        }
        if self.agent_loop_canary_bucket == eligible_migration_class {
            eligible_migration_class
        } else {
            "none"
        }
    }

    pub(super) fn enabled_rollout_switches(&self) -> Vec<&'static str> {
        let mut switches = Vec::new();
        match self.effective_answer_verifier_required_evidence_scope() {
            AnswerVerifierRequiredEvidenceScope::Off => {}
            AnswerVerifierRequiredEvidenceScope::SelectedAgentLoop
            | AnswerVerifierRequiredEvidenceScope::All => {
                switches.push("answer_verifier_enforce_required_scope")
            }
        }
        if self.effective_semantic_route_authority() != SemanticRouteAuthority::Legacy {
            switches.push("semantic_route_authority");
        }
        match self.effective_registry_idempotency_guard_scope() {
            RegistryIdempotencyGuardScope::Off => {}
            RegistryIdempotencyGuardScope::SelectedAgentLoop
            | RegistryIdempotencyGuardScope::All => {
                switches.push("registry_idempotency_guard_scope")
            }
        }
        if self.structured_evidence_required_for_selected_contracts {
            switches.push("structured_evidence_required_for_selected_contracts");
        }
        switches
    }

    pub(super) fn effective_answer_verifier_required_evidence_scope(
        &self,
    ) -> AnswerVerifierRequiredEvidenceScope {
        self.answer_verifier_enforce_required_scope
    }

    pub(super) fn answer_verifier_required_evidence_enabled_for_route(
        &self,
        route_result: Option<&crate::RouteResult>,
    ) -> bool {
        match self.effective_answer_verifier_required_evidence_scope() {
            AnswerVerifierRequiredEvidenceScope::Off => false,
            AnswerVerifierRequiredEvidenceScope::All => true,
            AnswerVerifierRequiredEvidenceScope::SelectedAgentLoop => {
                route_result.is_some_and(|route| self.selected_agent_loop_route(route))
            }
        }
    }

    pub(super) fn effective_registry_idempotency_guard_scope(
        &self,
    ) -> RegistryIdempotencyGuardScope {
        self.registry_idempotency_guard_scope
    }

    pub(super) fn registry_idempotency_guard_enabled_for_route(
        &self,
        route_result: Option<&crate::RouteResult>,
    ) -> bool {
        match self.effective_registry_idempotency_guard_scope() {
            RegistryIdempotencyGuardScope::Off => false,
            RegistryIdempotencyGuardScope::All => true,
            RegistryIdempotencyGuardScope::SelectedAgentLoop => {
                route_result.is_some_and(|route| self.selected_agent_loop_route(route))
            }
        }
    }

    fn selected_agent_loop_route(&self, route: &crate::RouteResult) -> bool {
        if !self.uses_agent_loop_semantic_authority()
            || route.risk_ceiling == crate::RiskCeiling::High
            || route.schedule_kind != crate::ScheduleKind::None
        {
            return false;
        }
        let eligible = super::migration_class::agent_decides_eligible_migration_class(route);
        self.selected_migration_class_for_eligible(eligible) != "none"
    }

    pub(super) fn budget_profile_for_context(
        recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
        route_result: Option<&crate::RouteResult>,
    ) -> LoopBudgetProfile {
        if matches!(
            recipe.kind,
            crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
        ) {
            return LoopBudgetProfile::OpsClosedLoop;
        }

        let Some(route) = route_result else {
            return LoopBudgetProfile::General;
        };
        let contract = crate::TaskContract::from_route_result(route);
        if !matches!(
            contract.intent_kind,
            crate::task_contract::TaskIntentKind::PlannerExecute
        ) {
            return LoopBudgetProfile::FastRead;
        }
        if route.output_contract.delivery_required
            || route.wants_file_delivery
            || matches!(
                contract.operation,
                crate::task_contract::TaskOperation::Write
                    | crate::task_contract::TaskOperation::Modify
                    | crate::task_contract::TaskOperation::Configure
            )
        {
            return LoopBudgetProfile::MultiStepWorkspace;
        }
        if matches!(
            contract.target_object,
            crate::task_contract::TaskTargetObject::Directory
        ) && matches!(
            contract.operation,
            crate::task_contract::TaskOperation::Summarize
        ) {
            return LoopBudgetProfile::MultiStepWorkspace;
        }
        if contract.required_evidence_fields.len() >= 2
            || (contract.evidence_required
                && matches!(
                    contract.operation,
                    crate::task_contract::TaskOperation::Summarize
                        | crate::task_contract::TaskOperation::Validate
                        | crate::task_contract::TaskOperation::Run
                        | crate::task_contract::TaskOperation::List
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
        route_result: Option<&crate::RouteResult>,
    ) -> Self {
        let profile = Self::budget_profile_for_context(recipe, route_result);
        let overrides = self.overrides_for_profile(profile);
        let mut policy = self.clone();
        if let Some(max_steps) = overrides.max_steps {
            policy.max_steps = max_steps;
        }
        if let Some(max_rounds) = overrides.max_rounds {
            policy.max_rounds = max_rounds;
        }
        if let Some(max_tool_calls) = overrides.max_tool_calls {
            policy.max_tool_calls = max_tool_calls;
        }
        if let Some(repeat_action_limit) = overrides.repeat_action_limit {
            policy.repeat_action_limit = repeat_action_limit;
        }
        if let Some(no_progress_limit) = overrides.no_progress_limit {
            policy.no_progress_limit = no_progress_limit;
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

fn parse_usize_allow_zero_from_toml(root: &TomlValue, path: &[&str], fallback: usize) -> usize {
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

fn parse_bool_from_toml(root: &TomlValue, path: &[&str], fallback: bool) -> bool {
    let mut cursor = root;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return fallback;
        };
        cursor = next;
    }
    cursor.as_bool().unwrap_or(fallback)
}

fn parse_agent_loop_canary_bucket(root: &TomlValue) -> String {
    const ALLOWED: &[&str] = &[
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
        "low_risk_single_file_delivery",
    ];
    let mut cursor = root;
    for key in ["agent", "loop_guard", "agent_loop_canary_bucket"] {
        let Some(next) = cursor.get(key) else {
            return "none".to_string();
        };
        cursor = next;
    }
    let value = cursor.as_str().unwrap_or("none").trim();
    if ALLOWED.contains(&value) {
        value.to_string()
    } else {
        "none".to_string()
    }
}

fn parse_semantic_route_authority(root: &TomlValue) -> Option<SemanticRouteAuthority> {
    let mut cursor = root;
    for key in ["agent", "loop_guard", "semantic_route_authority"] {
        cursor = cursor.get(key)?;
    }
    SemanticRouteAuthority::from_token(cursor.as_str().unwrap_or("legacy"))
}

fn parse_answer_verifier_required_evidence_scope(
    root: &TomlValue,
) -> Option<AnswerVerifierRequiredEvidenceScope> {
    let mut cursor = root;
    for key in [
        "agent",
        "loop_guard",
        "answer_verifier_enforce_required_scope",
    ] {
        cursor = cursor.get(key)?;
    }
    AnswerVerifierRequiredEvidenceScope::from_token(cursor.as_str().unwrap_or("off"))
}

fn parse_registry_idempotency_guard_scope(
    root: &TomlValue,
) -> Option<RegistryIdempotencyGuardScope> {
    let mut cursor = root;
    for key in ["agent", "loop_guard", "registry_idempotency_guard_scope"] {
        cursor = cursor.get(key)?;
    }
    RegistryIdempotencyGuardScope::from_token(cursor.as_str().unwrap_or("off"))
}

fn parse_loop_recipe_overrides(root: &TomlValue, path: &[&str]) -> LoopRecipeOverrides {
    let mut max_steps_path = path.to_vec();
    max_steps_path.push("max_steps");
    let mut max_rounds_path = path.to_vec();
    max_rounds_path.push("max_rounds");
    let mut max_tool_calls_path = path.to_vec();
    max_tool_calls_path.push("max_tool_calls");
    let mut repeat_action_limit_path = path.to_vec();
    repeat_action_limit_path.push("repeat_action_limit");
    let mut no_progress_limit_path = path.to_vec();
    no_progress_limit_path.push("no_progress_limit");
    let mut max_repairs_path = path.to_vec();
    max_repairs_path.push("max_repairs");
    let mut run_cmd_timeout_path = path.to_vec();
    run_cmd_timeout_path.push("run_cmd_timeout_seconds");
    let mut run_cmd_validation_timeout_path = path.to_vec();
    run_cmd_validation_timeout_path.push("run_cmd_validation_timeout_seconds");

    LoopRecipeOverrides {
        max_steps: parse_optional_usize_from_toml(root, &max_steps_path),
        max_rounds: parse_optional_usize_from_toml(root, &max_rounds_path),
        max_tool_calls: parse_optional_usize_from_toml(root, &max_tool_calls_path),
        repeat_action_limit: parse_optional_usize_from_toml(root, &repeat_action_limit_path),
        no_progress_limit: parse_optional_usize_from_toml(root, &no_progress_limit_path),
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
    let semantic_route_authority =
        parse_semantic_route_authority(&parsed).unwrap_or(SemanticRouteAuthority::Legacy);
    let answer_verifier_enforce_required_scope =
        parse_answer_verifier_required_evidence_scope(&parsed)
            .unwrap_or(AnswerVerifierRequiredEvidenceScope::Off);
    let registry_idempotency_guard_scope = parse_registry_idempotency_guard_scope(&parsed)
        .unwrap_or(RegistryIdempotencyGuardScope::Off);
    let policy = AgentLoopGuardPolicy {
        max_steps: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "max_steps"],
            crate::AGENT_MAX_STEPS,
        ),
        max_rounds: parse_usize_from_toml(&parsed, &["agent", "loop_guard", "max_rounds"], 2),
        max_tool_calls: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "max_tool_calls"],
            12,
        ),
        recoverable_failure_extra_rounds: parse_usize_allow_zero_from_toml(
            &parsed,
            &["agent", "loop_guard", "recoverable_failure_extra_rounds"],
            1,
        ),
        repeat_action_limit: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "repeat_action_limit"],
            4,
        ),
        no_progress_limit: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "no_progress_limit"],
            1,
        ),
        multi_round_enabled: parse_bool_from_toml(
            &parsed,
            &["agent", "loop_guard", "multi_round_enabled"],
            true,
        ),
        answer_verifier_retry_limit: parse_usize_allow_zero_from_toml(
            &parsed,
            &["agent", "loop_guard", "answer_verifier_retry_limit"],
            2,
        ),
        answer_verifier_enforce_required_scope,
        semantic_route_authority,
        agent_loop_canary_bucket: parse_agent_loop_canary_bucket(&parsed),
        registry_idempotency_guard_scope,
        structured_evidence_required_for_selected_contracts: parse_bool_from_toml(
            &parsed,
            &[
                "agent",
                "loop_guard",
                "structured_evidence_required_for_selected_contracts",
            ],
            false,
        ),
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
    if let Err(err) = repo::update_task_progress_result(state, &task.task_id, &payload.to_string())
    {
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
    let mut observations = loop_state
        .executed_step_results
        .iter()
        .rev()
        .take(8)
        .map(|step| {
            json!({
                "step_id": step.step_id,
                "skill": step.skill,
                "status": step.status.as_str(),
                "has_output": step.output.as_deref().is_some_and(|value| !value.trim().is_empty()),
                "has_error": step.error.as_deref().is_some_and(|value| !value.trim().is_empty()),
            })
        })
        .collect::<Vec<_>>();
    observations.reverse();
    observations
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

pub(super) fn build_agent_loop_checkpoint_progress_payload(
    task: &ClaimedTask,
    loop_state: &super::LoopState,
    resume_reason: &str,
    now_ts: i64,
    next_check_after: i64,
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
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({
            "schema_version": 1,
            "source": "agent_loop_soft_budget",
            "task_id": task.task_id,
            "resume_reason": resume_reason,
        }),
        last_successful_round: (loop_state.round_no > 0)
            .then_some(saturating_u32(loop_state.round_no)),
        last_successful_step,
        pending_action: None,
        observations: checkpoint_step_observations(loop_state),
        evidence_refs,
        artifact_refs: Vec::new(),
        completed_side_effect_refs: completed_side_effect_refs(loop_state),
        budget: CheckpointBudgetCounters {
            round: saturating_u32(loop_state.round_no),
            step: saturating_u32(loop_state.total_steps_executed),
            llm_calls: 0,
            tool_calls: saturating_u32(loop_state.tool_calls_total),
            elapsed_ms: 0,
        },
        pending_async_job: None,
        repair_signal: loop_state.last_stop_signal.as_ref().map(|signal| {
            json!({
                "kind": "agent_loop_stop_signal",
                "signal": signal,
            })
        }),
        resume_entrypoint: ResumeEntrypoint::NextPlannerRound,
    };

    json!({
        "progress_messages": loop_state.progress_messages,
        "task_lifecycle": {
            "schema_version": 1,
            "state": TaskLifecycleState::Waiting,
            "source": "agent_loop_soft_budget",
            "resume_reason": resume_reason,
            "next_check_after": next_check_after.max(now_ts + 1),
            "checkpoint_id": checkpoint_id,
            "can_poll": true,
            "can_cancel": true,
            "last_heartbeat_ts": now_ts,
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
    let payload = build_agent_loop_checkpoint_progress_payload(
        task,
        loop_state,
        resume_reason,
        now_ts,
        now_ts + 60,
    );
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
    if let Err(err) = repo::update_task_progress_result(state, &task.task_id, &payload.to_string())
    {
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
            let normalized_args = normalize_args_for_fingerprint(&normalized_skill, args);
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
            let normalized_args = normalize_args_for_fingerprint(&normalized_skill, args);
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
            let normalized_args = normalize_args_for_fingerprint(&normalized, args);
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
    route_result: Option<&crate::RouteResult>,
) -> String {
    if !policy.registry_idempotency_guard_enabled_for_route(route_result) {
        return action_fingerprint(state, action);
    }
    let Some((skill_name, args)) = action_skill_and_args(action) else {
        return action_fingerprint(state, action);
    };
    let normalized_skill = state
        .resolve_canonical_skill_name(skill_name)
        .to_ascii_lowercase();
    let action_token = registry_action_token_from_args(args);
    let Some(registry) = state.get_skills_registry() else {
        return action_fingerprint(state, action);
    };
    let once_per_task = registry.resolved_once_per_task(&normalized_skill, action_token.as_deref());
    let dedup_scope = registry.resolved_dedup_scope(&normalized_skill, action_token.as_deref());
    if once_per_task || dedup_scope == claw_core::skill_registry::RegistryDedupScope::Action {
        if literal_execution_failed_step_run_cmd_uses_args_fingerprint(
            &normalized_skill,
            action_token.as_deref(),
            args,
            route_result,
        ) {
            return action_fingerprint(state, action);
        }
        return format!(
            "skill:{}:action:{}",
            normalized_skill,
            action_token.unwrap_or_else(|| "_default".to_string())
        );
    }
    action_fingerprint(state, action)
}

pub(super) fn registry_idempotency_guard_attribution(
    state: &AppState,
    policy: &AgentLoopGuardPolicy,
    action: &AgentAction,
    route_result: Option<&crate::RouteResult>,
    fingerprint: &str,
    reason_code: &str,
    repeat_count: Option<usize>,
    limit: Option<usize>,
) -> Option<crate::task_journal::TaskJournalRolloutAttribution> {
    if !policy.registry_idempotency_guard_enabled_for_route(route_result) {
        return None;
    }
    let (skill_name, args) = action_skill_and_args(action)?;
    let normalized_skill = state
        .resolve_canonical_skill_name(skill_name)
        .to_ascii_lowercase();
    let action_token = registry_action_token_from_args(args);
    let registry = state.get_skills_registry()?;
    let once_per_task = registry.resolved_once_per_task(&normalized_skill, action_token.as_deref());
    let dedup_scope = registry.resolved_dedup_scope(&normalized_skill, action_token.as_deref());
    if !once_per_task && dedup_scope != claw_core::skill_registry::RegistryDedupScope::Action {
        return None;
    }
    if literal_execution_failed_step_run_cmd_uses_args_fingerprint(
        &normalized_skill,
        action_token.as_deref(),
        args,
        route_result,
    ) {
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

fn literal_execution_failed_step_run_cmd_uses_args_fingerprint(
    normalized_skill: &str,
    action_token: Option<&str>,
    args: &Value,
    route_result: Option<&crate::RouteResult>,
) -> bool {
    let is_run_command_action = normalized_skill == "run_cmd"
        || (normalized_skill == "system_basic" && action_token == Some("run_cmd"));
    if !is_run_command_action {
        return false;
    }
    if args
        .get(super::CLAWD_LITERAL_COMMAND_ARG)
        .and_then(Value::as_bool)
        != Some(true)
    {
        return false;
    }
    route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ExecutionFailedStep
    })
}

#[cfg(test)]
#[path = "support_tests.rs"]
mod tests;
fn normalize_run_cmd_command_for_fingerprint(command: &str) -> String {
    let tokens = command
        .split_whitespace()
        .map(normalize_command_token_for_fingerprint)
        .collect::<Vec<_>>();
    tokens.join(" ")
}

fn normalize_command_token_for_fingerprint(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    if token.starts_with('-') || token.contains('$') || token.contains('*') {
        return token.to_string();
    }
    if token.starts_with("./") || token.contains("/./") || token.contains("//") {
        return normalize_path_string_for_fingerprint(token);
    }
    token.to_string()
}

fn normalize_path_string_for_fingerprint(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    let mut quote_prefix = String::new();
    let mut quote_suffix = String::new();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        quote_prefix = s[..1].to_string();
        quote_suffix = s[s.len() - 1..].to_string();
        s = s[1..s.len().saturating_sub(1)].to_string();
    }

    while s.starts_with("./") {
        s = s[2..].to_string();
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s = s.replace("/./", "/");

    let path = Path::new(&s);
    let mut parts = Vec::new();
    let mut absolute = false;
    for comp in path.components() {
        match comp {
            Component::RootDir => absolute = true,
            Component::CurDir => {}
            Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            Component::ParentDir => parts.push("..".to_string()),
            Component::Prefix(_) => {}
        }
    }
    let mut out = if absolute {
        format!("/{}", parts.join("/"))
    } else {
        parts.join("/")
    };
    if out.is_empty() {
        out = ".".to_string();
    }
    format!("{quote_prefix}{out}{quote_suffix}")
}

fn normalize_args_for_fingerprint(action_name: &str, args: &Value) -> Value {
    let mut out = args.clone();
    if action_name == "run_cmd" {
        if let Some(obj) = out.as_object_mut() {
            if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
                obj.insert(
                    "command".to_string(),
                    Value::String(normalize_run_cmd_command_for_fingerprint(cmd)),
                );
            }
            if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
                obj.insert(
                    "cwd".to_string(),
                    Value::String(normalize_path_string_for_fingerprint(cwd)),
                );
            }
        }
    }
    out
}

fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    out.insert(key, canonicalize_json_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json_value).collect()),
        Value::Number(num) => canonicalize_json_number(num),
        _ => value.clone(),
    }
}

fn canonicalize_json_number(num: &serde_json::Number) -> Value {
    if num.is_i64() || num.is_u64() {
        return Value::Number(num.clone());
    }
    let Some(float_value) = num.as_f64() else {
        return Value::Number(num.clone());
    };
    if !float_value.is_finite() {
        return Value::Number(num.clone());
    }
    let rounded = float_value.round();
    if (float_value - rounded).abs() <= 1e-12 {
        if rounded >= 0.0 && rounded <= u64::MAX as f64 {
            return Value::Number(serde_json::Number::from(rounded as u64));
        }
        if rounded >= i64::MIN as f64 && rounded <= i64::MAX as f64 {
            return Value::Number(serde_json::Number::from(rounded as i64));
        }
    }
    Value::Number(num.clone())
}

fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(&canonicalize_json_value(value)).unwrap_or_else(|_| value.to_string())
}
