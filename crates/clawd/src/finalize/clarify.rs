use tracing::warn;

use crate::{fallback::ClarifyFallbackSource, llm_gateway, AppState, ClaimedTask};

const CLARIFY_QUESTION_PROMPT_LOGICAL_PATH: &str = "prompts/clarify_question_prompt.md";
const CLARIFICATION_COMPOSER_PERSONA: &str = "clarification_composer_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClarifyQuestionPolicy {
    #[default]
    AllowModel,
    SafeFallback,
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
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, request.user_request);
    if let Some(question) = request
        .preferred_question
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|question| {
            !crate::language_policy::text_language_conflicts_with_hint(question, &language_hint)
        })
    {
        return question.to_string();
    }

    if matches!(request.policy, ClarifyQuestionPolicy::SafeFallback) {
        if request.fallback_source == ClarifyFallbackSource::LlmUnavailable {
            return crate::fallback::render_clarify_fallback_with_language_hint(
                state,
                &task.task_id,
                request.fallback_source,
                None,
                &language_hint,
            );
        }
        let contract = crate::fallback::UserResponseContract::clarify_from_fallback_source(
            request.fallback_source,
            request.user_request,
            request.resolver_reason,
            request.candidate_context,
            &language_hint,
        );
        let default_text = crate::fallback::clarify_fallback_text_with_language_hint(
            state,
            request.fallback_source,
            None,
            &language_hint,
        );
        let answer = crate::fallback::compose_user_response_from_contract_with_default(
            state,
            task,
            &contract,
            request.fallback_source,
            &default_text,
        )
        .await;
        return if crate::language_policy::text_language_conflicts_with_hint(&answer, &language_hint)
        {
            default_text
        } else {
            answer
        };
    }

    generate_clarify_question(state, task, request, &language_hint).await
}

async fn generate_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    request: ClarifyRenderRequest<'_>,
    language_hint: &str,
) -> String {
    let (template, prompt_source) = match crate::bootstrap::load_required_prompt_template_for_state(
        state,
        CLARIFY_QUESTION_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            warn!(task_id = %task.task_id, %err, "clarify_prompt_load_failed");
            return crate::fallback::render_clarify_fallback_with_language_hint(
                state,
                &task.task_id,
                ClarifyFallbackSource::LlmUnavailable,
                Some(&err.to_string()),
                language_hint,
            );
        }
    };
    let prompt = crate::render_prompt_template(
        &template,
        &[
            ("__PERSONA_PROMPT__", CLARIFICATION_COMPOSER_PERSONA),
            ("__REQUEST__", request.user_request.trim()),
            ("__RESOLVER_REASON__", request.resolver_reason.trim()),
            ("__REQUEST_LANGUAGE_HINT__", language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            (
                "__CANDIDATE_CONTEXT__",
                request
                    .candidate_context
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("<none>"),
            ),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "clarify_question_prompt",
        &prompt_source,
        None,
    );
    match llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
        .await
    {
        Ok(answer) if !answer.trim().is_empty() => {
            let answer = answer.trim();
            if crate::language_policy::text_language_conflicts_with_hint(answer, language_hint) {
                clarify_fallback(state, task, request.fallback_source, language_hint, None)
            } else {
                answer.to_string()
            }
        }
        Ok(_) => clarify_fallback(
            state,
            task,
            ClarifyFallbackSource::EmptyResponse,
            language_hint,
            None,
        ),
        Err(err) => {
            warn!(task_id = %task.task_id, %err, "clarify_model_call_failed");
            clarify_fallback(
                state,
                task,
                ClarifyFallbackSource::LlmUnavailable,
                language_hint,
                Some(&format!("err={err}")),
            )
        }
    }
}

fn clarify_fallback(
    state: &AppState,
    task: &ClaimedTask,
    source: ClarifyFallbackSource,
    language_hint: &str,
    context_hint: Option<&str>,
) -> String {
    crate::fallback::render_clarify_fallback_with_language_hint(
        state,
        &task.task_id,
        source,
        context_hint,
        language_hint,
    )
}
