use serde_json::Value;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{OutputLocatorKind, OutputResponseShape, OutputSemanticKind};

pub(super) fn synthesize_bounded_read_range_direct_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.route_result.as_ref())?;
    if !route_allows_bounded_read_range_direct_answer(route) {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(bounded_read_range_answer_from_output)
}

fn route_allows_bounded_read_range_direct_answer(route: &crate::RouteResult) -> bool {
    let contract = route.effective_output_contract();
    !contract.delivery_required
        && matches!(
            contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::Strict
        )
        && matches!(
            contract.locator_kind,
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
        && route.output_contract_is_unclassified()
        && !route.output_contract_marker_is_any(&[
            OutputSemanticKind::ContentExcerptSummary,
            OutputSemanticKind::ContentExcerptWithSummary,
            OutputSemanticKind::ExcerptKindJudgment,
            OutputSemanticKind::RawCommandOutput,
            OutputSemanticKind::CommandOutputSummary,
        ])
}

fn bounded_read_range_answer_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    bounded_read_range_answer_from_value(&value)
}

fn bounded_read_range_answer_from_value(value: &Value) -> Option<String> {
    if let Some(answer) = bounded_read_range_answer_from_flat_value(value) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(bounded_read_range_answer_from_value)
}

fn bounded_read_range_answer_from_flat_value(value: &Value) -> Option<String> {
    if !matches!(
        value.get("action").and_then(Value::as_str),
        Some("read_range" | "read_text_range")
    ) || !matches!(
        value.get("mode").and_then(Value::as_str),
        Some("head" | "tail" | "range")
    ) {
        return None;
    }
    let requested_lines = value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(Value::as_u64)
        .or_else(|| {
            let start = value.get("start_line")?.as_u64()?;
            let end = value.get("end_line")?.as_u64()?;
            (end >= start).then_some(end - start + 1)
        })?;
    if requested_lines == 0 || requested_lines > 100 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(Value::as_str)
        .and_then(crate::agent_engine::observed_output::normalize_read_range_excerpt)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}
