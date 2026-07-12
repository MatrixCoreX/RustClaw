use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

pub(super) fn attach_config_edit_observed_answer_from_registry(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let direct_answer = latest_config_edit_terminal_machine_payload(loop_state)
        .map(|answer| (answer, config_edit_marker_replacement_summary()))
        .or_else(|| {
            super::config_edit::direct_config_edit_observed_answer(state, user_text, loop_state)
        });
    let rendered = if let Some((answer, summary)) = direct_answer {
        if loop_state.delivery_messages.is_empty() {
            *finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            super::log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_config_edit_observed",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            true
        } else if loop_delivery_has_replaceable_machine_marker(
            loop_state,
            &loop_state.delivery_messages,
        ) {
            *finalizer_summary = Some(summary);
            loop_state.delivery_messages.clear();
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            super::log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_config_edit_observed_marker_replace",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            true
        } else {
            false
        }
    } else {
        false
    };
    record_capability_result_renderer_trace(
        task,
        loop_state,
        "config_edit_observed_answer",
        rendered,
    );
    rendered
}

pub(super) fn replace_config_edit_machine_marker_delivery(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    if !loop_delivery_has_replaceable_machine_marker(loop_state, delivery_messages) {
        return false;
    }
    let Some((answer, summary)) =
        super::config_edit::direct_config_edit_observed_answer(state, user_text, loop_state)
    else {
        record_capability_result_renderer_trace(
            task,
            loop_state,
            "config_edit_observed_answer",
            false,
        );
        return false;
    };
    delivery_messages.clear();
    append_delivery_message(&task.task_id, delivery_messages, answer.clone());
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    super::log_deterministic_delivery_record(
        &task.task_id,
        "final_delivery_from_config_edit_observed_marker_replace",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    record_capability_result_renderer_trace(task, loop_state, "config_edit_observed_answer", true);
    true
}

pub(super) fn replace_config_edit_machine_marker_final_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let final_answer = super::final_answer_text_from_delivery(delivery_messages);
    if !text_is_replaceable_machine_marker(&final_answer) {
        return false;
    }
    if let Some(answer) = latest_config_edit_terminal_machine_payload(loop_state) {
        delivery_messages.clear();
        append_delivery_message(&task.task_id, delivery_messages, answer.clone());
        loop_state.delivery_messages = delivery_messages.clone();
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(config_edit_marker_replacement_summary());
        super::log_deterministic_delivery_record(
            &task.task_id,
            "final_answer_from_config_edit_terminal_machine_payload",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        record_capability_result_renderer_trace(
            task,
            loop_state,
            "config_edit_observed_answer",
            true,
        );
        return true;
    }
    let Some((answer, summary)) =
        super::config_edit::direct_config_edit_observed_answer(state, user_text, loop_state)
    else {
        record_capability_result_renderer_trace(
            task,
            loop_state,
            "config_edit_observed_answer",
            false,
        );
        return false;
    };
    delivery_messages.clear();
    append_delivery_message(&task.task_id, delivery_messages, answer.clone());
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    super::log_deterministic_delivery_record(
        &task.task_id,
        "final_answer_from_config_edit_observed_marker_replace",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    record_capability_result_renderer_trace(task, loop_state, "config_edit_observed_answer", true);
    true
}

fn latest_config_edit_terminal_machine_payload(loop_state: &LoopState) -> Option<String> {
    if let Some(answer) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .and_then(super::config_edit::config_edit_machine_payload_text)
    {
        return Some(answer);
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "synthesize_answer" | "respond")
        })
        .filter_map(|step| step.output.as_deref())
        .find_map(super::config_edit::config_edit_machine_payload_text)
}

fn config_edit_marker_replacement_summary() -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    }
}

fn delivery_is_replaceable_machine_marker(delivery_messages: &[String]) -> bool {
    let [message] = delivery_messages else {
        return false;
    };
    text_is_replaceable_machine_marker(message)
}

fn loop_delivery_has_replaceable_machine_marker(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    delivery_is_replaceable_machine_marker(delivery_messages)
        || loop_state
            .last_user_visible_respond
            .as_deref()
            .is_some_and(text_is_replaceable_machine_marker)
        || text_is_replaceable_machine_marker(&super::final_answer_text_from_delivery(
            delivery_messages,
        ))
}

fn text_is_replaceable_machine_marker(message: &str) -> bool {
    let message = message.trim();
    !message.is_empty()
        && !message.contains('=')
        && message
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            == 1
        && message
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

pub(super) fn run_service_status_observed_fields_renderer(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let rendered = super::replace_delivery_with_service_status_observed_answer(
        task,
        loop_state,
        agent_run_context,
        finalizer_summary,
    );
    record_capability_result_renderer_trace(
        task,
        loop_state,
        "service_status_observed_fields",
        rendered,
    );
    rendered
}

fn record_capability_result_renderer_trace(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    renderer_key: &'static str,
    rendered: bool,
) {
    let Some(renderer) = super::renderer_registry::renderers_for_shape_class(
        super::renderer_registry::FinalizerRendererShapeClass::CapabilityResult,
    )
    .find(|renderer| renderer.key == renderer_key) else {
        return;
    };
    super::renderer_registry::record_renderer_trace(
        loop_state,
        renderer,
        rendered,
        capability_result_renderer_evidence_refs(task, loop_state),
        (!rendered).then_some("not_applicable"),
    );
}

fn capability_result_renderer_evidence_refs(
    task: &ClaimedTask,
    loop_state: &LoopState,
) -> Vec<String> {
    let mut refs = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let mut reference = String::from("step_result:");
            reference.push_str(&index.to_string());
            reference
        })
        .collect::<Vec<_>>();
    if refs.is_empty() {
        refs.push(format!("task:{}", task.task_id));
    }
    refs
}
