use super::{route_trace::RouteTraceRecord, turn_analysis::TurnAnalysis};
use crate::{
    ActFinalizeStyle, IntentOutputContract, OutputLocatorKind, ResumeBehavior, ScheduleKind,
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
    pub(crate) boundary_envelope: BoundaryEnvelope,
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
    /// Execution finalization style. This is not a semantic gate.
    pub(crate) execution_finalize_style: ActFinalizeStyle,
    pub(crate) turn_analysis: Option<TurnAnalysis>,
    pub(crate) fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    pub(crate) route_trace_record: RouteTraceRecord,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BoundaryEnvelope {
    pub(crate) language_hint: Option<String>,
    pub(crate) schedule_intent: Option<crate::ScheduleIntentOutput>,
    pub(crate) attachment_refs: Vec<String>,
    pub(crate) explicit_locators: Vec<String>,
    pub(crate) active_task_reference: Option<String>,
    pub(crate) session_binding: Option<String>,
    pub(crate) safety_budget_hint: Option<String>,
    /// Request length only; never carry the raw natural-language request.
    pub(crate) raw_chars: usize,
}

impl BoundaryEnvelope {
    pub(crate) fn from_request(
        request: &str,
        schedule_intent: Option<crate::ScheduleIntentOutput>,
        attachment_processing_required: bool,
        output_contract: &IntentOutputContract,
        turn_analysis: Option<&TurnAnalysis>,
        resume_behavior: ResumeBehavior,
    ) -> Self {
        let language_hint = crate::language_policy::request_language_hint(request);
        Self {
            language_hint: (language_hint != "config_default").then(|| language_hint.to_string()),
            schedule_intent,
            attachment_refs: attachment_refs_for_boundary(attachment_processing_required),
            explicit_locators: explicit_locator_refs_for_boundary(output_contract),
            active_task_reference: turn_analysis
                .and_then(|analysis| analysis.target_task_policy)
                .map(|policy| policy.as_str().to_string()),
            session_binding: resume_behavior_boundary_token(resume_behavior).map(str::to_string),
            safety_budget_hint: None,
            raw_chars: request.chars().count(),
        }
    }

    pub(crate) fn raw_char_count(&self) -> usize {
        self.raw_chars
    }

    pub(crate) fn compact_prompt_line(&self) -> String {
        let schedule_kind = self
            .schedule_intent
            .as_ref()
            .map(|intent| non_empty_token(&intent.kind))
            .unwrap_or("none");
        format!(
            "- boundary_envelope raw_chars={} schedule_intent={} attachment_refs={} explicit_locators={} active_task_reference={} session_binding={} language_hint={} safety_budget_hint={}",
            self.raw_char_count(),
            schedule_kind,
            self.attachment_refs.len(),
            self.explicit_locators.len(),
            self.active_task_reference
                .as_deref()
                .map(non_empty_token)
                .unwrap_or("none"),
            self.session_binding
                .as_deref()
                .map(non_empty_token)
                .unwrap_or("none"),
            self.language_hint
                .as_deref()
                .map(non_empty_token)
                .unwrap_or("none"),
            self.safety_budget_hint
                .as_deref()
                .map(non_empty_token)
                .unwrap_or("none"),
        )
    }
}

impl IntentNormalizerOutput {
    pub(crate) fn boundary_envelope(&self) -> BoundaryEnvelope {
        self.boundary_envelope.clone()
    }
}

fn non_empty_token(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "none"
    } else {
        trimmed
    }
}

fn explicit_locator_refs_for_boundary(contract: &IntentOutputContract) -> Vec<String> {
    if contract.locator_kind == OutputLocatorKind::None {
        return Vec::new();
    }
    let locator = contract.locator_hint.trim();
    if locator.is_empty() {
        Vec::new()
    } else {
        vec![locator.to_string()]
    }
}

fn attachment_refs_for_boundary(required: bool) -> Vec<String> {
    if required {
        vec!["current_request_attachments".to_string()]
    } else {
        Vec::new()
    }
}

fn resume_behavior_boundary_token(resume_behavior: ResumeBehavior) -> Option<&'static str> {
    match resume_behavior {
        ResumeBehavior::None => None,
        ResumeBehavior::ResumeExecute => Some("resume_execute"),
        ResumeBehavior::ResumeDiscuss => Some("resume_discuss"),
    }
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
