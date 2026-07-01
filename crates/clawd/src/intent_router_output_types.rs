use super::{route_trace::RouteTraceRecord, turn_analysis::TurnAnalysis};
use crate::{
    ActFinalizeStyle, FirstLayerDecision, IntentOutputContract, ResumeBehavior, ScheduleKind,
};

#[derive(Debug, Clone)]
pub(crate) struct ContextResolution {
    pub(crate) resolved_user_intent: String,
    pub(crate) needs_clarify: bool,
    pub(crate) confidence: Option<f64>,
    pub(crate) reason: String,
}

/// Output of the unified intent normalizer (replaces resume_followup_intent + context_resolver + schedule_intent + intent_router in one LLM call).
#[derive(Debug, Clone)]
pub(crate) struct IntentNormalizerOutput {
    pub(crate) resolved_user_intent: String,
    pub(crate) resume_behavior: ResumeBehavior,
    pub(crate) schedule_kind: ScheduleKind,
    pub(crate) schedule_intent: Option<crate::ScheduleIntentOutput>,
    pub(crate) wants_file_delivery: bool,
    pub(crate) should_refresh_long_term_memory: bool,
    pub(crate) agent_display_name_hint: String,
    pub(crate) needs_clarify: bool,
    pub(crate) clarify_question: String,
    pub(crate) reason: String,
    pub(crate) confidence: f64,
    pub(crate) output_contract: IntentOutputContract,
    pub(crate) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(crate) execution_recipe_plan_hint: Option<ExecutionRecipePlanHint>,
    /// Trace-only route hint from the normalizer compatibility schema.
    ///
    /// Runtime dispatch must derive route authority from machine state such as
    /// `needs_clarify`, output contract, delivery, schedule, and execution recipe.
    #[allow(dead_code)]
    pub(crate) route_trace_decision: FirstLayerDecision,
    /// Execution finalization style. This is not a semantic gate.
    pub(crate) execution_finalize_style: ActFinalizeStyle,
    pub(crate) turn_analysis: Option<TurnAnalysis>,
    pub(crate) fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    pub(crate) route_trace_record: RouteTraceRecord,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExecutionRecipePlanHint {
    pub(crate) kind: String,
    pub(crate) command: Option<String>,
    pub(crate) execution_mode: Option<String>,
    pub(crate) async_adapter_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClarifyQuestionPolicy {
    #[default]
    AllowModel,
    SafeFallback,
}
