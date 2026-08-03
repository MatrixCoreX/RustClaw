pub(crate) fn planned_delivery_is_publishable_model_language_answer(delivery: &str) -> bool {
    let delivery = delivery.trim();
    !delivery.is_empty()
        && crate::finalize::parse_delivery_token(delivery).is_none()
        && !crate::finalize::looks_like_planner_artifact(delivery)
        && !crate::finalize::looks_like_internal_trace_artifact(delivery)
        && !crate::finalize::looks_like_structured_machine_output(delivery)
        && !crate::finalize::message_is_non_answer_separator(delivery)
}
