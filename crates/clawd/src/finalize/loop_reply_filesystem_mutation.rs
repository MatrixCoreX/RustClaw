use crate::agent_engine::{AgentRunContext, LoopState};
use crate::finalize::build_from_loop_state as build_loop_journal;
use crate::{AskReply, ClaimedTask};

use super::{route_accepts_filesystem_mutation_synthesis, valid_publishable_synthesis_output};

pub(super) fn filesystem_mutation_synthesis_reply(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    let synthesis = valid_publishable_synthesis_output(loop_state)?;
    if !route_accepts_filesystem_mutation_synthesis(route, synthesis) {
        return None;
    }
    let final_text = if route_prefers_status_line_for_filesystem_mutation(route) {
        filesystem_mutation_status_line(synthesis)?
    } else {
        synthesis.to_string()
    };
    let delivery_messages = vec![final_text.clone()];
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

fn route_prefers_status_line_for_filesystem_mutation(route: &crate::IntentOutputContract) -> bool {
    route.semantic_kind_is(crate::OutputSemanticKind::FilesystemMutationResult)
        && !route.delivery_required
        && !route.delivery_required
        && matches!(
            route.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
}

fn filesystem_mutation_status_line(synthesis: &str) -> Option<String> {
    let payload = serde_json::from_str::<serde_json::Value>(synthesis.trim()).ok()?;
    let mut parts = vec!["status=ok".to_string()];
    push_status_line_part(
        &mut parts,
        "effective_status",
        first_string_from_payload(&payload, &["effective_status"], &["effective_status"]),
    );
    push_status_line_part(
        &mut parts,
        "result_kind",
        first_string_from_payload(&payload, &["result_kind", "result_kinds"], &["result_kind"]),
    );
    push_status_line_part(
        &mut parts,
        "action",
        first_string_from_payload(&payload, &["action", "observed_actions"], &["action"]),
    );
    push_status_line_part(
        &mut parts,
        "path",
        first_string_from_payload(&payload, &["path", "paths", "resolved_path"], &["path"]),
    );
    push_status_line_part(
        &mut parts,
        "namespace",
        first_string_from_payload(&payload, &["namespace", "namespaces"], &["namespace"]),
    );
    if let Some(total_chunks) = payload
        .pointer("/steps/0/stats/total_chunks")
        .and_then(serde_json::Value::as_u64)
    {
        parts.push(format!("total_chunks={total_chunks}"));
    }
    Some(parts.join(" "))
}

fn first_string_from_payload(
    payload: &serde_json::Value,
    top_level_keys: &[&str],
    step_keys: &[&str],
) -> Option<String> {
    first_string_from_object(payload, top_level_keys).or_else(|| {
        payload
            .pointer("/steps/0")
            .and_then(|step| first_string_from_object(step, step_keys))
    })
}

fn first_string_from_object(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        if let Some(text) = value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
        value
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find_map(|item| item.as_str().map(str::trim).filter(|text| !text.is_empty()))
            })
            .map(ToString::to_string)
    })
}

fn push_status_line_part(parts: &mut Vec<String>, key: &str, value: Option<String>) {
    let Some(value) = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    parts.push(format!("{key}={}", status_line_value(value)));
}

fn status_line_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '/' | '@'))
    {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
    }
}
