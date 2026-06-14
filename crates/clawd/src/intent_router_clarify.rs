use tracing::warn;

use crate::{llm_gateway, schedule_service, AppState, ClaimedTask};

use super::{
    ClarifyQuestionPolicy, CLARIFY_QUESTION_PROMPT_LOGICAL_PATH, ROUTING_POLICY_PERSONA_PROMPT,
};

pub(crate) async fn generate_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resolver_reason: &str,
    candidate_context: Option<&str>,
) -> String {
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let (prompt_template, prompt_source) =
        match crate::bootstrap::load_required_prompt_template_for_state(
            state,
            CLARIFY_QUESTION_PROMPT_LOGICAL_PATH,
        ) {
            Ok(resolved) => resolved,
            Err(err) => {
                warn!(
                    "generate_clarify_question prompt load failed, fallback default: task_id={} err={}",
                    task.task_id, err
                );
                return crate::fallback::render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    crate::fallback::ClarifyFallbackSource::LlmUnavailable,
                    Some(&err.to_string()),
                    &request_language_hint,
                );
            }
        };
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__REQUEST__", user_request.trim()),
            ("__RESOLVER_REASON__", resolver_reason.trim()),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            (
                "__CANDIDATE_CONTEXT__",
                candidate_context
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
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
        Ok(v) => {
            let out = v.trim();
            if out.is_empty() {
                // §7.2: LLM 调用 OK 但内容为空 → EmptyResponse 特化文案 + tracing 上报。
                crate::fallback::render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    crate::fallback::ClarifyFallbackSource::EmptyResponse,
                    None,
                    &request_language_hint,
                )
            } else {
                out.to_string()
            }
        }
        Err(err) => {
            warn!(
                "generate_clarify_question llm failed, fallback default: task_id={} err={}",
                task.task_id, err
            );
            // §7.2: LLM 直接 Err（401 / 熔断 / 超时 / 网络）→ LlmUnavailable 特化文案。
            // err 概要写进 context_hint，便于 inspect_task.sh 关联。
            let hint = format!("err={err}");
            crate::fallback::render_clarify_fallback_with_language_hint(
                state,
                &task.task_id,
                crate::fallback::ClarifyFallbackSource::LlmUnavailable,
                Some(&hint),
                &request_language_hint,
            )
        }
    }
}

pub(crate) async fn generate_or_reuse_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resolver_reason: &str,
    candidate_context: Option<&str>,
    preferred_question: Option<&str>,
    policy: ClarifyQuestionPolicy,
    // 上游必须显式声明"我现在为什么走 SafeFallback"。
    // policy=SafeFallback 时优先让 LLM 按结构化上下文生成用户可见澄清；
    // 只有 LLM 本身不可用等硬失败才落到该 source 的最小安全模板。
    // policy=AllowModel 时本参数仅作为诊断上下文。
    default_source: crate::fallback::ClarifyFallbackSource,
) -> String {
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let preferred = preferred_question
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .filter(|question| {
            !crate::language_policy::text_language_conflicts_with_hint(
                question,
                &request_language_hint,
            )
        })
        .map(ToString::to_string);
    if let Some(question) = preferred {
        return question;
    }
    if matches!(policy, ClarifyQuestionPolicy::SafeFallback)
        && !safe_fallback_source_should_try_llm(default_source)
    {
        return crate::fallback::render_clarify_fallback_with_language_hint(
            state,
            &task.task_id,
            default_source,
            None,
            &request_language_hint,
        );
    }
    if matches!(policy, ClarifyQuestionPolicy::SafeFallback) {
        tracing::info!(
            task_id = %task.task_id,
            fallback_source = default_source.as_metric_label(),
            "safe_fallback_try_llm_response_composer"
        );
        let contract = crate::fallback::UserResponseContract::clarify_from_fallback_source(
            default_source,
            user_request,
            resolver_reason,
            candidate_context,
            &request_language_hint,
        );
        let default_text = crate::fallback::clarify_fallback_text_with_language_hint(
            state,
            default_source,
            None,
            &request_language_hint,
        );
        let answer = crate::fallback::compose_user_response_from_contract_with_default(
            state,
            task,
            &contract,
            default_source,
            &default_text,
        )
        .await;
        if crate::language_policy::text_language_conflicts_with_hint(
            &answer,
            &request_language_hint,
        ) {
            tracing::info!(
                task_id = %task.task_id,
                fallback_source = default_source.as_metric_label(),
                language_hint = %request_language_hint,
                "clarify_generated_language_mismatch_fallback"
            );
            return default_text;
        }
        return answer;
    }
    let answer = generate_clarify_question(
        state,
        task,
        user_request,
        resolver_reason,
        candidate_context,
    )
    .await;
    if crate::language_policy::text_language_conflicts_with_hint(&answer, &request_language_hint) {
        return crate::fallback::render_clarify_fallback_with_language_hint(
            state,
            &task.task_id,
            default_source,
            None,
            &request_language_hint,
        );
    }
    answer
}

pub(super) fn safe_fallback_source_should_try_llm(
    source: crate::fallback::ClarifyFallbackSource,
) -> bool {
    !matches!(
        source,
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    )
}

pub(crate) async fn try_handle_schedule_request(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    precompiled_intent: Option<&crate::ScheduleIntentOutput>,
) -> Result<Option<String>, String> {
    schedule_service::try_handle_schedule_request(state, task, prompt, precompiled_intent).await
}
