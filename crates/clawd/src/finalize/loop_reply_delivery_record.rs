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
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let semantic_kind = route
        .map(|route| format!("{:?}", route.output_contract.semantic_kind))
        .unwrap_or_else(|| "None".to_string());
    let response_shape = route
        .map(|route| format!("{:?}", route.output_contract.response_shape))
        .unwrap_or_else(|| "None".to_string());
    let delivery_required = route
        .map(|route| route.output_contract.delivery_required)
        .unwrap_or(false);
    let content_evidence = route
        .map(|route| route.output_contract.requires_content_evidence)
        .unwrap_or(false);
    info!(
        "deterministic_delivery_record task_id={} owner_layer={} reason_code={} outcome={} semantic_kind={} response_shape={} delivery_required={} content_evidence={} evidence_count={}",
        task_id,
        FINALIZER_DETERMINISTIC_OWNER,
        reason_code,
        outcome,
        semantic_kind,
        response_shape,
        delivery_required,
        content_evidence,
        evidence_count,
    );
}
