use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use toml::Value as TomlValue;

const TASK_BUDGET_SCHEMA_VERSION: u32 = 1;

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
    pub(crate) ops_closed_loop: bool,
}

pub(crate) fn profile_for_verified_plan(facts: VerifiedPlanBudgetFacts) -> TaskBudgetProfile {
    if facts.ops_closed_loop {
        return TaskBudgetProfile::OpsClosedLoop;
    }
    if facts.delivery_required
        || facts.has_continuation
        || facts.needs_confirmation
        || facts.mutate_count > 0
        || facts.action_count >= 4
    {
        return TaskBudgetProfile::MultiStepWorkspace;
    }
    if facts.evidence_required
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
    pub(crate) artifact_count: u64,
    pub(crate) completed_plan_nodes: u64,
    pub(crate) verified_state_transitions: u64,
    pub(crate) async_continuations: u64,
    pub(crate) stagnation_count: u32,
}

impl BudgetProgress {
    pub(crate) fn observed_progress(&self) -> bool {
        self.evidence_count > 0
            || self.artifact_count > 0
            || self.completed_plan_nodes > 0
            || self.verified_state_transitions > 0
            || self.async_continuations > 0
    }

    fn advanced_from(&self, previous: &Self) -> bool {
        self.evidence_count > previous.evidence_count
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
}

impl TaskBudgetSlice {
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
        }
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
        let decision = evaluate_budget_decision(self, &observation, progress_advanced);
        self.cumulative_model_turns = observation.cumulative_model_turns;
        self.cumulative_tool_calls = observation.cumulative_tool_calls;
        self.cumulative_input_tokens = observation.cumulative_input_tokens;
        self.cumulative_output_tokens = observation.cumulative_output_tokens;
        self.cumulative_cost_usd_nanos = observation.cumulative_cost_usd_nanos;
        self.cumulative_elapsed_ms = observation.cumulative_elapsed_ms;
        self.progress = observation.progress;
        self.last_decision = decision;
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
    observation.cumulative_model_turns >= slice.hard_ceilings.model_turns
        || observation.cumulative_tool_calls >= slice.hard_ceilings.tool_calls
        || observation
            .cumulative_input_tokens
            .saturating_add(observation.cumulative_output_tokens)
            >= slice.hard_ceilings.total_tokens
        || observation.cumulative_cost_usd_nanos >= slice.hard_ceilings.cost_usd_nanos
        || observation.cumulative_elapsed_ms >= slice.hard_ceilings.elapsed_ms
        || slice.continuation_index >= slice.hard_ceilings.continuations
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
