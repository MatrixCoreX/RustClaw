use crate::agent_engine::LoopState;

pub(super) fn direct_observed_count_answer_for_scalar_contract(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_prefers_observed_count(route, loop_state) {
        return None;
    }
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok()
                || matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
            {
                return None;
            }
            let output = step
                .output
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())?;
            let body =
                crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                    output,
                );
            serde_json::from_str::<serde_json::Value>(body.trim())
                .ok()
                .and_then(|value| observed_count_from_value(&value))
        })?;
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
            ..Default::default()
        },
    ))
}

fn route_prefers_observed_count(route: &crate::RouteResult, loop_state: &LoopState) -> bool {
    let contract = route.effective_output_contract();
    if contract.delivery_required {
        return false;
    }
    route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        && !plan_requests_count_inventory_file_dir_breakdown(loop_state)
}

fn plan_requests_count_inventory_file_dir_breakdown(loop_state: &LoopState) -> bool {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|trace| trace.plan_result.as_ref())
        .any(|plan| {
            plan.steps.iter().any(|step| {
                step.is_skill_invocation()
                    && step.skill == "system_basic"
                    && step.args.get("action").and_then(|value| value.as_str())
                        == Some("count_inventory")
                    && step
                        .args
                        .get("count_files")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    && step
                        .args
                        .get("count_dirs")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
            })
        })
}

fn observed_count_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if let Some(count) = observed_count_from_value(extra) {
            return Some(count);
        }
    }
    if path_batch_facts_has_missing_target(value) {
        return None;
    }
    numeric_count(value.get("count"))
        .or_else(|| numeric_count(value.pointer("/counts/total")))
        .or_else(|| numeric_count(value.pointer("/summary/count")))
}

fn path_batch_facts_has_missing_target(value: &serde_json::Value) -> bool {
    value.get("action").and_then(|value| value.as_str()) == Some("path_batch_facts")
        && value
            .get("facts")
            .and_then(|value| value.as_array())
            .is_some_and(|facts| {
                facts.iter().any(|fact| {
                    fact.get("exists")
                        .and_then(|value| value.as_bool())
                        .is_some_and(|exists| !exists)
                })
            })
}

fn numeric_count(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                return Some(value.to_string());
            }
            number
                .as_i64()
                .filter(|value| *value >= 0)
                .map(|value| value.to_string())
        }
        serde_json::Value::String(text)
            if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            Some(text.clone())
        }
        _ => None,
    }
}
