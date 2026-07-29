use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use toml::Value as TomlValue;

const TASK_BUDGET_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BudgetAllocationKind {
    ModelTurn,
    ChildTask,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BudgetUnits {
    pub(crate) model_turns: u64,
    pub(crate) tool_calls: u64,
    pub(crate) tokens: u64,
    pub(crate) elapsed_ms: u64,
}

impl BudgetUnits {
    fn saturating_sub(&self, consumed: &Self) -> Self {
        Self {
            model_turns: self.model_turns.saturating_sub(consumed.model_turns),
            tool_calls: self.tool_calls.saturating_sub(consumed.tool_calls),
            tokens: self.tokens.saturating_sub(consumed.tokens),
            elapsed_ms: self.elapsed_ms.saturating_sub(consumed.elapsed_ms),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BudgetAllocation {
    pub(crate) allocation_id: String,
    pub(crate) owner_ref: String,
    pub(crate) kind: BudgetAllocationKind,
    pub(crate) granted: BudgetUnits,
    #[serde(default)]
    pub(crate) consumed: BudgetUnits,
    #[serde(default)]
    pub(crate) reclaimed: BudgetUnits,
    #[serde(default)]
    pub(crate) settled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskBudgetProfile {
    General,
    FastRead,
    GroundedSummary,
    MultiStepWorkspace,
    OpsClosedLoop,
}

impl TaskBudgetProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::FastRead => "fast_read",
            Self::GroundedSummary => "grounded_summary",
            Self::MultiStepWorkspace => "multi_step_workspace",
            Self::OpsClosedLoop => "ops_closed_loop",
        }
    }

    pub(crate) fn widen_with(self, candidate: Self) -> Self {
        if candidate.widening_rank() > self.widening_rank() {
            candidate
        } else {
            self
        }
    }

    fn widening_rank(self) -> u8 {
        match self {
            Self::FastRead => 0,
            Self::General => 1,
            Self::GroundedSummary => 2,
            Self::MultiStepWorkspace => 3,
            Self::OpsClosedLoop => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BudgetDecision {
    Continue,
    Finish,
    CheckpointRequeue,
    Waiting,
    NeedsUser,
    Terminal,
}

impl BudgetDecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Finish => "finish",
            Self::CheckpointRequeue => "checkpoint_requeue",
            Self::Waiting => "waiting",
            Self::NeedsUser => "needs_user",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BudgetTimeoutClass {
    Short,
    Standard,
    LongTail,
}

impl BudgetTimeoutClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Standard => "standard",
            Self::LongTail => "long_tail",
        }
    }

    fn call_ceiling_seconds(self) -> u64 {
        match self {
            Self::Short => 60,
            Self::Standard => 180,
            Self::LongTail => 900,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BudgetHardCeilings {
    pub(crate) model_turns: u64,
    pub(crate) tool_calls: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cost_usd_nanos: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) continuations: u32,
    pub(crate) non_resumable_tool_runtime_ms: u64,
}

impl Default for BudgetHardCeilings {
    fn default() -> Self {
        Self {
            model_turns: 256,
            tool_calls: 512,
            total_tokens: 100_000_000,
            cost_usd_nanos: 100_000_000_000,
            elapsed_ms: 24 * 60 * 60 * 1_000,
            continuations: 64,
            non_resumable_tool_runtime_ms: 60 * 60 * 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BudgetProfilePolicy {
    pub(crate) soft_slice_ms: u64,
    pub(crate) stagnation_tolerance: u32,
    pub(crate) provider_timeout_class: BudgetTimeoutClass,
    pub(crate) tool_timeout_class: BudgetTimeoutClass,
}

impl BudgetProfilePolicy {
    fn default_for(profile: TaskBudgetProfile) -> Self {
        match profile {
            TaskBudgetProfile::General => Self {
                soft_slice_ms: 300_000,
                stagnation_tolerance: 3,
                provider_timeout_class: BudgetTimeoutClass::Standard,
                tool_timeout_class: BudgetTimeoutClass::Standard,
            },
            TaskBudgetProfile::FastRead => Self {
                soft_slice_ms: 120_000,
                stagnation_tolerance: 2,
                provider_timeout_class: BudgetTimeoutClass::Short,
                tool_timeout_class: BudgetTimeoutClass::Short,
            },
            TaskBudgetProfile::GroundedSummary => Self {
                soft_slice_ms: 300_000,
                stagnation_tolerance: 3,
                provider_timeout_class: BudgetTimeoutClass::Standard,
                tool_timeout_class: BudgetTimeoutClass::Standard,
            },
            TaskBudgetProfile::MultiStepWorkspace => Self {
                soft_slice_ms: 900_000,
                stagnation_tolerance: 4,
                provider_timeout_class: BudgetTimeoutClass::Standard,
                tool_timeout_class: BudgetTimeoutClass::LongTail,
            },
            TaskBudgetProfile::OpsClosedLoop => Self {
                soft_slice_ms: 1_200_000,
                stagnation_tolerance: 4,
                provider_timeout_class: BudgetTimeoutClass::Standard,
                tool_timeout_class: BudgetTimeoutClass::LongTail,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskBudgetPolicy {
    pub(crate) hard_ceilings: BudgetHardCeilings,
    general: BudgetProfilePolicy,
    fast_read: BudgetProfilePolicy,
    grounded_summary: BudgetProfilePolicy,
    multi_step_workspace: BudgetProfilePolicy,
    ops_closed_loop: BudgetProfilePolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct VerifiedPlanBudgetFacts {
    pub(crate) action_count: usize,
    pub(crate) observe_count: usize,
    pub(crate) mutate_count: usize,
    pub(crate) validate_count: usize,
    pub(crate) needs_confirmation: bool,
    pub(crate) evidence_required: bool,
    pub(crate) delivery_required: bool,
    pub(crate) has_continuation: bool,
    pub(crate) required_tool_timeout_seconds: u64,
    pub(crate) has_long_tail_action: bool,
    pub(crate) ops_closed_loop: bool,
}

pub(crate) fn profile_for_verified_plan(facts: VerifiedPlanBudgetFacts) -> TaskBudgetProfile {
    if facts.ops_closed_loop {
        return TaskBudgetProfile::OpsClosedLoop;
    }
    if facts.delivery_required
        || facts.has_continuation
        || facts.has_long_tail_action
        || facts.required_tool_timeout_seconds > BudgetTimeoutClass::Standard.call_ceiling_seconds()
        || facts.needs_confirmation
        || facts.mutate_count > 0
        || facts.action_count >= 4
    {
        return TaskBudgetProfile::MultiStepWorkspace;
    }
    if facts.evidence_required
        || facts.required_tool_timeout_seconds > BudgetTimeoutClass::Short.call_ceiling_seconds()
        || facts.validate_count > 0
        || facts.observe_count >= 2
        || facts.action_count >= 2
    {
        return TaskBudgetProfile::GroundedSummary;
    }
    if facts.action_count == 1 {
        return TaskBudgetProfile::FastRead;
    }
    TaskBudgetProfile::General
}

impl Default for TaskBudgetPolicy {
    fn default() -> Self {
        Self {
            hard_ceilings: BudgetHardCeilings::default(),
            general: BudgetProfilePolicy::default_for(TaskBudgetProfile::General),
            fast_read: BudgetProfilePolicy::default_for(TaskBudgetProfile::FastRead),
            grounded_summary: BudgetProfilePolicy::default_for(TaskBudgetProfile::GroundedSummary),
            multi_step_workspace: BudgetProfilePolicy::default_for(
                TaskBudgetProfile::MultiStepWorkspace,
            ),
            ops_closed_loop: BudgetProfilePolicy::default_for(TaskBudgetProfile::OpsClosedLoop),
        }
    }
}

impl TaskBudgetPolicy {
    pub(crate) fn profile(&self, profile: TaskBudgetProfile) -> BudgetProfilePolicy {
        match profile {
            TaskBudgetProfile::General => self.general,
            TaskBudgetProfile::FastRead => self.fast_read,
            TaskBudgetProfile::GroundedSummary => self.grounded_summary,
            TaskBudgetProfile::MultiStepWorkspace => self.multi_step_workspace,
            TaskBudgetProfile::OpsClosedLoop => self.ops_closed_loop,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BudgetProgress {
    pub(crate) evidence_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) machine_progress_digest: Option<String>,
    pub(crate) artifact_count: u64,
    pub(crate) completed_plan_nodes: u64,
    pub(crate) verified_state_transitions: u64,
    pub(crate) async_continuations: u64,
    pub(crate) stagnation_count: u32,
}

impl BudgetProgress {
    pub(crate) fn observed_progress(&self) -> bool {
        self.evidence_count > 0
            || self.machine_progress_digest.is_some()
            || self.artifact_count > 0
            || self.completed_plan_nodes > 0
            || self.verified_state_transitions > 0
            || self.async_continuations > 0
    }

    fn advanced_from(&self, previous: &Self) -> bool {
        self.evidence_count > previous.evidence_count
            || self
                .machine_progress_digest
                .as_deref()
                .filter(|digest| !digest.is_empty())
                .is_some_and(|digest| previous.machine_progress_digest.as_deref() != Some(digest))
            || self.artifact_count > previous.artifact_count
            || self.completed_plan_nodes > previous.completed_plan_nodes
            || self.verified_state_transitions > previous.verified_state_transitions
            || self.async_continuations > previous.async_continuations
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskBudgetSlice {
    pub(crate) schema_version: u32,
    pub(crate) profile: TaskBudgetProfile,
    pub(crate) soft_slice_ms: u64,
    pub(crate) stagnation_tolerance: u32,
    pub(crate) provider_timeout_class: BudgetTimeoutClass,
    pub(crate) tool_timeout_class: BudgetTimeoutClass,
    pub(crate) continuation_index: u32,
    pub(crate) cumulative_model_turns: u64,
    pub(crate) cumulative_tool_calls: u64,
    pub(crate) cumulative_input_tokens: u64,
    pub(crate) cumulative_output_tokens: u64,
    pub(crate) cumulative_cost_usd_nanos: u64,
    pub(crate) cumulative_elapsed_ms: u64,
    pub(crate) progress: BudgetProgress,
    pub(crate) hard_ceilings: BudgetHardCeilings,
    pub(crate) last_decision: BudgetDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_limit_hit: Option<claw_core::adaptive_limits::LimitHit>,
    /// Persisted reservations owned by the single task-budget authority.
    /// Settled entries remain as an audit ledger; their unused units are
    /// reclaimed and therefore do not reduce future allocations.
    #[serde(default)]
    pub(crate) allocations: Vec<BudgetAllocation>,
    #[serde(default)]
    pub(crate) delegated_consumed: BudgetUnits,
}

impl TaskBudgetSlice {
    pub(crate) fn provider_call_timeout_seconds(&self) -> u64 {
        call_timeout_seconds(self.provider_timeout_class, self.soft_slice_ms, u64::MAX)
    }

    pub(crate) fn tool_call_timeout_seconds(&self) -> u64 {
        call_timeout_seconds(
            self.tool_timeout_class,
            self.soft_slice_ms,
            self.hard_ceilings.non_resumable_tool_runtime_ms,
        )
    }

    #[cfg(test)]
    pub(crate) fn new(
        profile: TaskBudgetProfile,
        soft_slice_ms: u64,
        hard_ceilings: BudgetHardCeilings,
    ) -> Self {
        let mut profile_policy = BudgetProfilePolicy::default_for(profile);
        profile_policy.soft_slice_ms = soft_slice_ms.max(1);
        Self::new_with_policy(profile, profile_policy, hard_ceilings)
    }

    pub(crate) fn new_with_policy(
        profile: TaskBudgetProfile,
        profile_policy: BudgetProfilePolicy,
        hard_ceilings: BudgetHardCeilings,
    ) -> Self {
        Self {
            schema_version: TASK_BUDGET_SCHEMA_VERSION,
            profile,
            soft_slice_ms: profile_policy.soft_slice_ms.max(1),
            stagnation_tolerance: profile_policy.stagnation_tolerance.max(1),
            provider_timeout_class: profile_policy.provider_timeout_class,
            tool_timeout_class: profile_policy.tool_timeout_class,
            continuation_index: 0,
            cumulative_model_turns: 0,
            cumulative_tool_calls: 0,
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cost_usd_nanos: 0,
            cumulative_elapsed_ms: 0,
            progress: BudgetProgress::default(),
            hard_ceilings,
            last_decision: BudgetDecision::Continue,
            last_limit_hit: None,
            allocations: Vec::new(),
            delegated_consumed: BudgetUnits::default(),
        }
    }

    pub(crate) fn allocate(
        &mut self,
        allocation_id: impl Into<String>,
        owner_ref: impl Into<String>,
        kind: BudgetAllocationKind,
        requested: BudgetUnits,
    ) -> Option<BudgetAllocation> {
        let allocation_id = allocation_id.into();
        if allocation_id.trim().is_empty()
            || self
                .allocations
                .iter()
                .any(|allocation| allocation.allocation_id == allocation_id)
        {
            return None;
        }
        let remaining = self.remaining_units();
        let granted = BudgetUnits {
            model_turns: requested.model_turns.min(remaining.model_turns),
            tool_calls: requested.tool_calls.min(remaining.tool_calls),
            tokens: requested.tokens.min(remaining.tokens),
            elapsed_ms: requested.elapsed_ms.min(remaining.elapsed_ms),
        };
        if (requested.model_turns > 0 && granted.model_turns == 0)
            || (requested.tool_calls > 0 && granted.tool_calls == 0)
            || (requested.tokens > 0 && granted.tokens == 0)
            || (requested.elapsed_ms > 0 && granted.elapsed_ms == 0)
        {
            return None;
        }
        let allocation = BudgetAllocation {
            allocation_id,
            owner_ref: owner_ref.into(),
            kind,
            granted,
            consumed: BudgetUnits::default(),
            reclaimed: BudgetUnits::default(),
            settled: false,
        };
        self.allocations.push(allocation.clone());
        Some(allocation)
    }

    pub(crate) fn remaining_units(&self) -> BudgetUnits {
        let reserved = self.active_reserved_units();
        let consumed_tokens = self
            .cumulative_input_tokens
            .saturating_add(self.cumulative_output_tokens);
        BudgetUnits {
            model_turns: self
                .hard_ceilings
                .model_turns
                .saturating_sub(self.cumulative_model_turns)
                .saturating_sub(self.delegated_consumed.model_turns)
                .saturating_sub(reserved.model_turns),
            tool_calls: self
                .hard_ceilings
                .tool_calls
                .saturating_sub(self.cumulative_tool_calls)
                .saturating_sub(self.delegated_consumed.tool_calls)
                .saturating_sub(reserved.tool_calls),
            tokens: self
                .hard_ceilings
                .total_tokens
                .saturating_sub(consumed_tokens)
                .saturating_sub(self.delegated_consumed.tokens)
                .saturating_sub(reserved.tokens),
            elapsed_ms: self
                .hard_ceilings
                .elapsed_ms
                .saturating_sub(self.cumulative_elapsed_ms)
                .saturating_sub(self.delegated_consumed.elapsed_ms)
                .saturating_sub(reserved.elapsed_ms),
        }
    }

    pub(crate) fn settle_allocation(&mut self, allocation_id: &str, consumed: BudgetUnits) -> bool {
        let Some(allocation) = self
            .allocations
            .iter_mut()
            .find(|allocation| allocation.allocation_id == allocation_id && !allocation.settled)
        else {
            return false;
        };
        allocation.consumed = consumed;
        allocation.reclaimed = allocation.granted.saturating_sub(&allocation.consumed);
        allocation.settled = true;
        if allocation.kind == BudgetAllocationKind::ChildTask {
            self.delegated_consumed.model_turns = self
                .delegated_consumed
                .model_turns
                .saturating_add(allocation.consumed.model_turns);
            self.delegated_consumed.tool_calls = self
                .delegated_consumed
                .tool_calls
                .saturating_add(allocation.consumed.tool_calls);
            self.delegated_consumed.tokens = self
                .delegated_consumed
                .tokens
                .saturating_add(allocation.consumed.tokens);
            self.delegated_consumed.elapsed_ms = self
                .delegated_consumed
                .elapsed_ms
                .saturating_add(allocation.consumed.elapsed_ms);
        }
        true
    }

    pub(crate) fn active_reserved_units(&self) -> BudgetUnits {
        self.allocations
            .iter()
            .filter(|allocation| !allocation.settled)
            .fold(BudgetUnits::default(), |mut total, allocation| {
                total.model_turns = total
                    .model_turns
                    .saturating_add(allocation.granted.model_turns);
                total.tool_calls = total
                    .tool_calls
                    .saturating_add(allocation.granted.tool_calls);
                total.tokens = total.tokens.saturating_add(allocation.granted.tokens);
                total.elapsed_ms = total
                    .elapsed_ms
                    .saturating_add(allocation.granted.elapsed_ms);
                total
            })
    }

    pub(crate) fn apply_profile(
        &mut self,
        profile: TaskBudgetProfile,
        mut profile_policy: BudgetProfilePolicy,
        worker_soft_limit_ms: u64,
    ) {
        profile_policy.soft_slice_ms = profile_policy
            .soft_slice_ms
            .min(worker_soft_limit_ms.max(1));
        self.profile = profile;
        self.soft_slice_ms = profile_policy.soft_slice_ms.max(1);
        self.stagnation_tolerance = profile_policy.stagnation_tolerance.max(1);
        self.provider_timeout_class = profile_policy.provider_timeout_class;
        self.tool_timeout_class = profile_policy.tool_timeout_class;
    }

    pub(crate) fn resumed(mut self) -> Self {
        self.continuation_index = self.continuation_index.saturating_add(1);
        self.last_decision = BudgetDecision::Continue;
        self
    }

    pub(crate) fn set_decision(&mut self, decision: BudgetDecision) {
        self.last_decision = decision;
    }

    pub(crate) fn observe(&mut self, observation: BudgetObservation) -> BudgetDecision {
        let progress_advanced = observation.progress.advanced_from(&self.progress);
        let limit_hit = administrator_limit_hit(self, &observation);
        let decision = evaluate_budget_decision(self, &observation, progress_advanced);
        self.cumulative_model_turns = observation.cumulative_model_turns;
        self.cumulative_tool_calls = observation.cumulative_tool_calls;
        self.cumulative_input_tokens = observation.cumulative_input_tokens;
        self.cumulative_output_tokens = observation.cumulative_output_tokens;
        self.cumulative_cost_usd_nanos = observation.cumulative_cost_usd_nanos;
        self.cumulative_elapsed_ms = observation.cumulative_elapsed_ms;
        self.progress = observation.progress;
        self.last_decision = decision;
        self.last_limit_hit = limit_hit;
        decision
    }

    pub(crate) fn to_machine_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            json!({
                "schema_version": TASK_BUDGET_SCHEMA_VERSION,
                "last_decision": BudgetDecision::Terminal.as_str(),
                "error_code": "task_budget_serialize_failed",
            })
        })
    }

    pub(crate) fn from_machine_json(value: &Value) -> Option<Self> {
        let slice = serde_json::from_value::<Self>(value.clone()).ok()?;
        (slice.schema_version == TASK_BUDGET_SCHEMA_VERSION).then_some(slice)
    }
}

fn call_timeout_seconds(
    timeout_class: BudgetTimeoutClass,
    soft_slice_ms: u64,
    administrator_ceiling_ms: u64,
) -> u64 {
    let soft_reserve_ms = soft_slice_ms.saturating_sub(1_000).max(1_000);
    let effective_ms = timeout_class
        .call_ceiling_seconds()
        .saturating_mul(1_000)
        .min(soft_reserve_ms)
        .min(administrator_ceiling_ms.max(1_000));
    effective_ms.saturating_add(999) / 1_000
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BudgetObservation {
    pub(crate) cumulative_model_turns: u64,
    pub(crate) cumulative_tool_calls: u64,
    pub(crate) cumulative_input_tokens: u64,
    pub(crate) cumulative_output_tokens: u64,
    pub(crate) cumulative_cost_usd_nanos: u64,
    pub(crate) cumulative_elapsed_ms: u64,
    pub(crate) progress: BudgetProgress,
    pub(crate) model_finished: bool,
    pub(crate) needs_user: bool,
    pub(crate) waiting: bool,
    pub(crate) cancelled: bool,
    pub(crate) policy_terminal: bool,
    pub(crate) stagnation_exhausted: bool,
    pub(crate) resumable: bool,
    pub(crate) soft_slice_exhausted: bool,
}

pub(crate) fn evaluate_budget_decision(
    slice: &TaskBudgetSlice,
    observation: &BudgetObservation,
    progress_advanced: bool,
) -> BudgetDecision {
    if observation.cancelled
        || observation.policy_terminal
        || hard_ceiling_reached(slice, observation)
    {
        return BudgetDecision::Terminal;
    }
    if observation.needs_user {
        return BudgetDecision::NeedsUser;
    }
    if observation.waiting {
        return BudgetDecision::Waiting;
    }
    if observation.model_finished {
        return BudgetDecision::Finish;
    }
    if observation.stagnation_exhausted && !progress_advanced {
        return BudgetDecision::Terminal;
    }
    if observation.soft_slice_exhausted {
        return if observation.resumable {
            BudgetDecision::CheckpointRequeue
        } else {
            BudgetDecision::Terminal
        };
    }
    BudgetDecision::Continue
}

fn hard_ceiling_reached(slice: &TaskBudgetSlice, observation: &BudgetObservation) -> bool {
    administrator_limit_hit(slice, observation).is_some()
}

fn administrator_limit_hit(
    slice: &TaskBudgetSlice,
    observation: &BudgetObservation,
) -> Option<claw_core::adaptive_limits::LimitHit> {
    use claw_core::adaptive_limits::{
        LimitClass, LimitHit, LimitRecovery, LimitUnit, LIMIT_HIT_SCHEMA_VERSION,
    };

    let total_model_turns = observation
        .cumulative_model_turns
        .saturating_add(slice.delegated_consumed.model_turns);
    let total_tool_calls = observation
        .cumulative_tool_calls
        .saturating_add(slice.delegated_consumed.tool_calls);
    let boundary = if total_model_turns >= slice.hard_ceilings.model_turns {
        (
            LimitUnit::Calls,
            slice.hard_ceilings.model_turns,
            total_model_turns,
            "administrator_model_turn_budget_exhausted",
        )
    } else if total_tool_calls >= slice.hard_ceilings.tool_calls {
        (
            LimitUnit::Calls,
            slice.hard_ceilings.tool_calls,
            total_tool_calls,
            "administrator_tool_call_budget_exhausted",
        )
    } else {
        let total_tokens = observation
            .cumulative_input_tokens
            .saturating_add(observation.cumulative_output_tokens)
            .saturating_add(slice.delegated_consumed.tokens);
        if total_tokens >= slice.hard_ceilings.total_tokens {
            (
                LimitUnit::Tokens,
                slice.hard_ceilings.total_tokens,
                total_tokens,
                "administrator_token_budget_exhausted",
            )
        } else if observation.cumulative_cost_usd_nanos >= slice.hard_ceilings.cost_usd_nanos {
            (
                LimitUnit::CostUsdNanos,
                slice.hard_ceilings.cost_usd_nanos,
                observation.cumulative_cost_usd_nanos,
                "administrator_cost_budget_exhausted",
            )
        } else if observation
            .cumulative_elapsed_ms
            .saturating_add(slice.delegated_consumed.elapsed_ms)
            >= slice.hard_ceilings.elapsed_ms
        {
            (
                LimitUnit::Milliseconds,
                slice.hard_ceilings.elapsed_ms,
                observation
                    .cumulative_elapsed_ms
                    .saturating_add(slice.delegated_consumed.elapsed_ms),
                "administrator_elapsed_budget_exhausted",
            )
        } else if slice.continuation_index >= slice.hard_ceilings.continuations {
            (
                LimitUnit::Continuations,
                u64::from(slice.hard_ceilings.continuations),
                u64::from(slice.continuation_index),
                "administrator_continuation_budget_exhausted",
            )
        } else {
            return None;
        }
    };

    Some(LimitHit {
        schema_version: LIMIT_HIT_SCHEMA_VERSION,
        class: LimitClass::TaskResource,
        owner: "task_budget_manager".to_string(),
        unit: boundary.0,
        configured_value: boundary.1,
        observed_value: boundary.2,
        reason_code: boundary.3.to_string(),
        terminal: true,
        recovery: LimitRecovery::None,
    })
}

pub(crate) fn load_task_budget_policy(workspace_root: &Path) -> TaskBudgetPolicy {
    let path = workspace_root.join("configs/agent_guard.toml");
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<TomlValue>(&raw).ok())
        .unwrap_or(TomlValue::Table(Default::default()));
    task_budget_policy_from_toml(&parsed)
}

fn task_budget_policy_from_toml(root: &TomlValue) -> TaskBudgetPolicy {
    let mut policy = TaskBudgetPolicy::default();
    policy.hard_ceilings.model_turns = parse_u64(
        root,
        &["agent", "task_budget", "admin_max_model_turns"],
        policy.hard_ceilings.model_turns,
    );
    policy.hard_ceilings.tool_calls = parse_u64(
        root,
        &["agent", "task_budget", "admin_max_tool_calls"],
        policy.hard_ceilings.tool_calls,
    );
    policy.hard_ceilings.total_tokens = parse_u64(
        root,
        &["agent", "task_budget", "admin_max_total_tokens"],
        policy.hard_ceilings.total_tokens,
    );
    policy.hard_ceilings.cost_usd_nanos = parse_u64(
        root,
        &["agent", "task_budget", "admin_max_cost_usd_nanos"],
        policy.hard_ceilings.cost_usd_nanos,
    );
    policy.hard_ceilings.elapsed_ms = parse_seconds_as_ms(
        root,
        &["agent", "task_budget", "admin_max_elapsed_seconds"],
        policy.hard_ceilings.elapsed_ms,
    );
    policy.hard_ceilings.continuations = parse_u64(
        root,
        &["agent", "task_budget", "admin_max_continuations"],
        u64::from(policy.hard_ceilings.continuations),
    )
    .min(u64::from(u32::MAX)) as u32;
    policy.hard_ceilings.non_resumable_tool_runtime_ms = parse_seconds_as_ms(
        root,
        &[
            "agent",
            "task_budget",
            "admin_max_non_resumable_tool_seconds",
        ],
        policy.hard_ceilings.non_resumable_tool_runtime_ms,
    );
    policy.general = parse_profile_policy(root, "general", TaskBudgetProfile::General);
    policy.fast_read = parse_profile_policy(root, "fast_read", TaskBudgetProfile::FastRead);
    policy.grounded_summary =
        parse_profile_policy(root, "grounded_summary", TaskBudgetProfile::GroundedSummary);
    policy.multi_step_workspace = parse_profile_policy(
        root,
        "multi_step_workspace",
        TaskBudgetProfile::MultiStepWorkspace,
    );
    policy.ops_closed_loop =
        parse_profile_policy(root, "ops_closed_loop", TaskBudgetProfile::OpsClosedLoop);
    policy
}

fn parse_profile_policy(
    root: &TomlValue,
    profile_token: &str,
    profile: TaskBudgetProfile,
) -> BudgetProfilePolicy {
    let mut policy = BudgetProfilePolicy::default_for(profile);
    policy.soft_slice_ms = parse_seconds_as_ms(
        root,
        &[
            "agent",
            "task_budget",
            "profiles",
            profile_token,
            "soft_slice_seconds",
        ],
        policy.soft_slice_ms,
    );
    policy.stagnation_tolerance = parse_u64(
        root,
        &[
            "agent",
            "task_budget",
            "profiles",
            profile_token,
            "stagnation_tolerance",
        ],
        u64::from(policy.stagnation_tolerance),
    )
    .min(u64::from(u32::MAX)) as u32;
    policy.provider_timeout_class = parse_timeout_class(
        root,
        profile_token,
        "provider_timeout_class",
        policy.provider_timeout_class,
    );
    policy.tool_timeout_class = parse_timeout_class(
        root,
        profile_token,
        "tool_timeout_class",
        policy.tool_timeout_class,
    );
    policy
}

fn parse_timeout_class(
    root: &TomlValue,
    profile_token: &str,
    key: &str,
    fallback: BudgetTimeoutClass,
) -> BudgetTimeoutClass {
    match value_at(
        root,
        &["agent", "task_budget", "profiles", profile_token, key],
    )
    .and_then(TomlValue::as_str)
    {
        Some("short") => BudgetTimeoutClass::Short,
        Some("standard") => BudgetTimeoutClass::Standard,
        Some("long_tail") => BudgetTimeoutClass::LongTail,
        _ => fallback,
    }
}

fn parse_seconds_as_ms(root: &TomlValue, path: &[&str], fallback_ms: u64) -> u64 {
    parse_u64(root, path, fallback_ms.saturating_add(999) / 1_000).saturating_mul(1_000)
}

fn parse_u64(root: &TomlValue, path: &[&str], fallback: u64) -> u64 {
    value_at(root, path)
        .and_then(TomlValue::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value >= 1)
        .unwrap_or(fallback)
}

fn value_at<'a>(root: &'a TomlValue, path: &[&str]) -> Option<&'a TomlValue> {
    let mut cursor = root;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    Some(cursor)
}

#[cfg(test)]
#[path = "task_budget_contract_tests.rs"]
mod tests;
