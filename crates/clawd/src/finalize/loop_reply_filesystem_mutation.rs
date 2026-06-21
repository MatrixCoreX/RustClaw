use crate::agent_engine::{AgentRunContext, LoopState};
use crate::finalize::build_from_loop_state as build_loop_journal;
use crate::{AskReply, ClaimedTask};

use super::{
    build_execution_summary_messages, route_accepts_filesystem_mutation_synthesis,
    valid_publishable_synthesis_output,
};

pub(super) fn filesystem_mutation_synthesis_reply(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let synthesis = valid_publishable_synthesis_output(loop_state)?;
    if !route_accepts_filesystem_mutation_synthesis(route, synthesis) {
        return None;
    }
    let mut delivery_messages =
        build_execution_summary_messages(loop_state, agent_run_context, Some(user_text));
    delivery_messages.push(synthesis.to_string());
    let final_text = synthesis.to_string();
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&final_text, &delivery_messages);
    let finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &final_text,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    Some(
        AskReply::non_llm(final_text)
            .with_messages(delivery_messages)
            .with_task_journal(journal),
    )
}
