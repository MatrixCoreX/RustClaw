use claw_core::capability_result::CapabilityResultStatus;

pub(crate) fn planned_delivery_is_publishable_model_language_answer(delivery: &str) -> bool {
    let delivery = delivery.trim();
    !delivery.is_empty()
        && crate::finalize::parse_delivery_token(delivery).is_none()
        && !crate::finalize::looks_like_planner_artifact(delivery)
        && !crate::finalize::looks_like_internal_trace_artifact(delivery)
        && !crate::finalize::looks_like_structured_machine_output(delivery)
        && !crate::finalize::message_is_non_answer_separator(delivery)
}

pub(crate) fn terminal_respond_after_failed_observation_is_publishable(
    loop_state: &crate::agent_engine::LoopState,
    delivery_messages: &[String],
) -> bool {
    let Some(delivery) = delivery_messages.last().map(String::as_str).map(str::trim) else {
        return false;
    };
    if !(planned_delivery_is_publishable_model_language_answer(delivery)
        || structured_terminal_respond_is_grounded_in_failed_result(delivery, loop_state))
        || loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            != Some(delivery)
        || !loop_state.executed_step_results.iter().any(|step| {
            !step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "think" | "synthesize_answer"
                )
        })
    {
        return false;
    }
    loop_state
        .executed_step_results
        .last()
        .filter(|step| step.is_ok() && step.skill == "respond")
        .and_then(|step| step.output.as_deref())
        .map(str::trim)
        == Some(delivery)
}

fn structured_terminal_respond_is_grounded_in_failed_result(
    delivery: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> bool {
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(delivery.trim())
    else {
        return false;
    };
    loop_state.capability_results.iter().rev().any(|result| {
        result.status == CapabilityResultStatus::Error
            && crate::capability_result::exact_object_projection_from_result(result, &object)
                .is_some()
    })
}
