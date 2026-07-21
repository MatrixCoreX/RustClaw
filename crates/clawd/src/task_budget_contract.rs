use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BudgetHardCeilings {
    pub(crate) model_turns: u64,
    pub(crate) tool_calls: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) continuations: u32,
}

impl Default for BudgetHardCeilings {
    fn default() -> Self {
        Self {
            model_turns: 256,
            tool_calls: 512,
            elapsed_ms: 24 * 60 * 60 * 1_000,
            continuations: 64,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskBudgetSlice {
    pub(crate) schema_version: u32,
    pub(crate) profile: TaskBudgetProfile,
    pub(crate) soft_slice_ms: u64,
    pub(crate) continuation_index: u32,
    pub(crate) cumulative_model_turns: u64,
    pub(crate) cumulative_tool_calls: u64,
    pub(crate) cumulative_elapsed_ms: u64,
    pub(crate) progress: BudgetProgress,
    pub(crate) hard_ceilings: BudgetHardCeilings,
    pub(crate) last_decision: BudgetDecision,
}

impl TaskBudgetSlice {
    pub(crate) fn new(
        profile: TaskBudgetProfile,
        soft_slice_ms: u64,
        hard_ceilings: BudgetHardCeilings,
    ) -> Self {
        Self {
            schema_version: TASK_BUDGET_SCHEMA_VERSION,
            profile,
            soft_slice_ms: soft_slice_ms.max(1),
            continuation_index: 0,
            cumulative_model_turns: 0,
            cumulative_tool_calls: 0,
            cumulative_elapsed_ms: 0,
            progress: BudgetProgress::default(),
            hard_ceilings,
            last_decision: BudgetDecision::Continue,
        }
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
        let decision = evaluate_budget_decision(self, &observation);
        self.cumulative_model_turns = observation.cumulative_model_turns;
        self.cumulative_tool_calls = observation.cumulative_tool_calls;
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
    pub(crate) cumulative_elapsed_ms: u64,
    pub(crate) progress: BudgetProgress,
    pub(crate) model_finished: bool,
    pub(crate) needs_user: bool,
    pub(crate) waiting: bool,
    pub(crate) cancelled: bool,
    pub(crate) policy_terminal: bool,
    pub(crate) resumable: bool,
    pub(crate) soft_slice_exhausted: bool,
}

pub(crate) fn evaluate_budget_decision(
    slice: &TaskBudgetSlice,
    observation: &BudgetObservation,
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
        || observation.cumulative_elapsed_ms >= slice.hard_ceilings.elapsed_ms
        || slice.continuation_index >= slice.hard_ceilings.continuations
}

#[cfg(test)]
#[path = "task_budget_contract_tests.rs"]
mod tests;
