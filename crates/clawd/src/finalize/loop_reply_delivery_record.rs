use tracing::info;

use crate::agent_engine::AgentRunContext;

const FINALIZER_DETERMINISTIC_OWNER: &str = "finalizer_deterministic_delivery";

pub(super) fn log_deterministic_delivery_record(
    task_id: &str,
    reason_code: &'static str,
    outcome: &'static str,
    agent_run_context: Option<&AgentRunContext>,
    evidence_count: usize,
) {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract());
    let final_answer_shape =
        route.and_then(crate::evidence_policy::final_answer_shape_for_output_contract);
    let final_answer_shape_token = final_answer_shape
        .map(crate::evidence_policy::FinalAnswerShape::as_str)
        .unwrap_or("none");
    let final_answer_shape_class = final_answer_shape
        .map(|shape| shape.class().as_str())
        .unwrap_or("none");
    let response_shape = route
        .map(|route| format!("{:?}", route.response_shape))
        .unwrap_or_else(|| "None".to_string());
    let delivery_required = route.map(|route| route.delivery_required).unwrap_or(false);
    let content_evidence = route
        .map(|route| route.requires_content_evidence)
        .unwrap_or(false);
    info!(
        "deterministic_delivery_record task_id={} owner_layer={} reason_code={} outcome={} final_answer_shape={} final_answer_shape_class={} response_shape={} delivery_required={} content_evidence={} evidence_count={}",
        task_id,
        FINALIZER_DETERMINISTIC_OWNER,
        reason_code,
        outcome,
        final_answer_shape_token,
        final_answer_shape_class,
        response_shape,
        delivery_required,
        content_evidence,
        evidence_count,
    );
}
