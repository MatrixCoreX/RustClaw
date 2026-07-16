use serde_json::Value;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

use super::log_deterministic_delivery_record;

pub(super) fn replace_delivery_with_weather_query_fields(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_is_weather_query(route) {
        return false;
    }
    let Some(answer) = latest_weather_query_field_projection(loop_state) else {
        return false;
    };
    if delivery_messages
        .iter()
        .any(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        return true;
    }
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
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
    log_deterministic_delivery_record(
        &task.task_id,
        "weather_query_structured_fields",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn route_is_weather_query(route: &crate::RouteResult) -> bool {
    route
        .output_contract
        .semantic_kind_is(crate::OutputSemanticKind::WeatherQuery)
}

fn latest_weather_query_field_projection(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "weather")
        .filter_map(|step| step.output.as_deref())
        .filter_map(weather_query_field_projection_from_output)
        .next()
}

fn weather_query_field_projection_from_output(output: &str) -> Option<String> {
    let payload = serde_json::from_str::<Value>(output.trim()).ok()?;
    let extra = payload.get("extra")?;
    let location = scalar_value_text(extra.get("location")?)?;
    let temperature = scalar_value_text(extra.get("temperature")?)?;
    let weather_code = scalar_value_text(extra.get("weather_code")?)?;
    Some(format!(
        "location={location}\ntemperature={temperature}\nweather_code={weather_code}"
    ))
}

fn scalar_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
