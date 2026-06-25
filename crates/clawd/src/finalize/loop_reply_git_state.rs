use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{current_user_visible_delivery_text, log_deterministic_delivery_record};

fn route_git_repository_state_requires_language_synthesis(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::GitRepositoryState
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        ) || route.output_contract.exact_sentence_count.is_some())
}

pub(super) async fn replace_git_repository_state_machine_delivery_with_observed_synthesis(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_git_repository_state_requires_language_synthesis(route) {
        return false;
    }
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if !crate::agent_engine::observed_output::answer_is_git_repository_state_machine_summary(
        current_delivery,
    ) {
        return false;
    }
    match crate::agent_engine::observed_output::try_synthesize_answer_from_observed_output(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    )
    .await
    {
        Ok(Some((answer, summary)))
            if matches!(
                summary.disposition,
                Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
            ) && !answer.trim().is_empty()
                && !crate::agent_engine::observed_output::answer_is_git_repository_state_machine_summary(
                    &answer,
                ) =>
        {
            loop_state
                .delivery_messages
                .retain(|message| crate::finalize::is_execution_summary_message(message));
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                answer.clone(),
            );
            loop_state.last_user_visible_respond = Some(answer);
            *finalizer_summary = Some(summary);
            log_deterministic_delivery_record(
                &task.task_id,
                "replace_git_machine_summary_with_observed_synthesis",
                "replaced",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            true
        }
        Ok(Some((_answer, summary))) => {
            if finalizer_summary.is_none() {
                *finalizer_summary = Some(summary);
            }
            false
        }
        Ok(None) => false,
        Err(err) => {
            tracing::warn!(
                "git_machine_summary_observed_synthesis_failed task_id={} err={}",
                task.task_id,
                err
            );
            false
        }
    }
}
