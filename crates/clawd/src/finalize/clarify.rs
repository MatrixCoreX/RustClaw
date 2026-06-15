use crate::{
    fallback::ClarifyFallbackSource, intent_router::ClarifyQuestionPolicy, AppState, ClaimedTask,
};

pub(crate) fn clarify_fallback_source_or_default(
    source: Option<ClarifyFallbackSource>,
) -> ClarifyFallbackSource {
    source.unwrap_or(ClarifyFallbackSource::IntentUnresolved)
}

pub(crate) struct ClarifyRenderRequest<'a> {
    pub(crate) user_request: &'a str,
    pub(crate) resolver_reason: &'a str,
    pub(crate) candidate_context: Option<&'a str>,
    pub(crate) preferred_question: Option<&'a str>,
    pub(crate) policy: ClarifyQuestionPolicy,
    pub(crate) fallback_source: ClarifyFallbackSource,
}

pub(crate) async fn render_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    request: ClarifyRenderRequest<'_>,
) -> String {
    crate::intent_router::generate_or_reuse_clarify_question(
        state,
        task,
        request.user_request,
        request.resolver_reason,
        request.candidate_context,
        request.preferred_question,
        request.policy,
        request.fallback_source,
    )
    .await
}
