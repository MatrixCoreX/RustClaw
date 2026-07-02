pub(super) fn apply_requested_machine_kv_summary_to_final_answer(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    if route_preserves_generated_file_machine_report(route_result, answer_text, answer_messages) {
        return false;
    }
    if final_answer_preserves_weather_query_machine_report(
        route_result,
        answer_text,
        answer_messages,
    ) {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    let Some(summary) = requested_machine_kv_summary_from_task_final_answer(
        prompt,
        route_result,
        journal,
        answer_text,
        answer_messages,
    ) else {
        return false;
    };
    if answer_text.trim() == summary {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if final_answer_preserves_compare_paths_existence_fields(answer_text, answer_messages)
        && !text_has_compare_paths_existence_fields(&summary)
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if final_answer_preserves_transform_structured_delivery(journal, answer_text, answer_messages) {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if final_answer_preserves_web_search_listing(
        route_result,
        journal,
        answer_text,
        answer_messages,
    ) {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if final_answer_has_values_for_requested_marker_summary(answer_text, answer_messages, &summary)
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    answer_messages.clear();
    answer_messages.push(summary.clone());
    *answer_text = summary;
    journal.record_final_answer(answer_text.as_str());
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: journal.step_results.len(),
        ..Default::default()
    });
    true
}

fn final_answer_preserves_weather_query_machine_report(
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    if !route_is_weather_query(route_result) {
        return false;
    }
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(weather_query_machine_fields_are_complete)
}

fn weather_query_machine_fields_are_complete(text: &str) -> bool {
    ["location", "temperature", "weather_code"]
        .into_iter()
        .all(|field| text_has_clean_line_value_for_marker(text, field))
}

fn text_has_clean_line_value_for_marker(text: &str, marker: &str) -> bool {
    text.lines().map(str::trim).any(|line| {
        let value = line
            .strip_prefix(format!("{marker}=").as_str())
            .or_else(|| line.strip_prefix(format!("{marker}:").as_str()))
            .map(str::trim);
        value.is_some_and(|value| {
            !value.is_empty()
                && !value.contains('\n')
                && !value.contains(" location=")
                && !value.contains(" temperature=")
                && !value.contains(" weather_code=")
        })
    })
}

fn final_answer_preserves_transform_structured_delivery(
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok && step.skill == "transform"
    }) && std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|text| text_is_markdown_table(text) || text_is_json_object_or_array(text))
}

fn text_is_markdown_table(text: &str) -> bool {
    let rows = text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('|') && line.ends_with('|'))
        .collect::<Vec<_>>();
    rows.len() >= 3
        && rows
            .iter()
            .any(|row| row.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')))
}

fn text_is_json_object_or_array(text: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(text.trim()) {
        Ok(serde_json::Value::Array(_)) | Ok(serde_json::Value::Object(_)) => true,
        _ => false,
    }
}

fn final_answer_preserves_web_search_listing(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    if !route_is_web_search_listing(route_result) {
        return false;
    }
    let visible = std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if visible.trim().is_empty() {
        return false;
    }
    journal
        .step_results
        .iter()
        .filter(|step| {
            step.status == crate::executor::StepExecutionStatus::Ok
                && matches!(step.skill.as_str(), "web_search_extract" | "browser_web")
        })
        .filter_map(|step| step.output_excerpt.as_deref())
        .flat_map(web_search_candidate_title_sources_from_output)
        .any(|(title, _source)| visible.contains(&title))
}

fn route_is_weather_query(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_namespace(route, &["weather"])
}

fn route_is_web_search_listing(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action(
        route,
        &["web", "browser"],
        &["search", "results"],
    )
}

fn web_search_candidate_title_sources_from_output(output: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    collect_web_search_candidate_title_sources_from_json(&value, &mut pairs);
    pairs
}

fn collect_web_search_candidate_title_sources_from_json(
    value: &serde_json::Value,
    pairs: &mut Vec<(String, String)>,
) {
    for pointer in [
        "/extra/candidates",
        "/extra/items",
        "/candidates",
        "/items",
        "/results",
    ] {
        if let Some(items) = value.pointer(pointer).and_then(serde_json::Value::as_array) {
            collect_web_search_candidate_array_title_sources(items, pairs);
        }
    }
}

fn collect_web_search_candidate_array_title_sources(
    items: &[serde_json::Value],
    pairs: &mut Vec<(String, String)>,
) {
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let title = object
            .get("title")
            .or_else(|| object.get("name"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let source = object
            .get("source")
            .or_else(|| object.get("domain"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let (Some(title), Some(source)) = (title, source) {
            pairs.push((title.to_string(), source.to_string()));
        }
    }
}

fn final_answer_has_values_for_requested_marker_summary(
    answer_text: &str,
    answer_messages: &[String],
    requested_summary: &str,
) -> bool {
    let requested_markers = bare_machine_markers(requested_summary);
    !requested_markers.is_empty()
        && std::iter::once(answer_text)
            .chain(answer_messages.iter().map(String::as_str))
            .any(|candidate| {
                requested_markers
                    .iter()
                    .all(|marker| text_has_value_for_marker(candidate, marker))
            })
}

fn bare_machine_markers(text: &str) -> Vec<String> {
    let tokens = text
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() || tokens.iter().any(|token| token.contains('=')) {
        return Vec::new();
    }
    tokens
        .into_iter()
        .filter(|token| valid_machine_marker_key(token))
        .map(str::to_string)
        .collect()
}

fn text_has_value_for_marker(text: &str, marker: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(format!("{marker}=").as_str()) {
            return !value.trim().is_empty();
        }
        if let Some(value) = line.strip_prefix(format!("{marker}:").as_str()) {
            return !value.trim().is_empty();
        }
        false
    })
}

fn valid_machine_marker_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn requested_machine_kv_summary_from_task_final_answer(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    let mut observed_texts =
        crate::machine_kv_projection::observed_machine_text_fragments_from_journal(journal);
    crate::machine_kv_projection::collect_machine_text_fragments_from_output(
        answer_text,
        &mut observed_texts,
    );
    for message in answer_messages {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    observed_texts.sort();
    observed_texts.dedup();
    let request_surfaces = task_machine_kv_request_surfaces(prompt, route_result, journal);
    if route_result.output_contract_marker_is(crate::OutputSemanticKind::ServiceStatus) {
        let service_control_texts =
            observed_machine_text_fragments_from_journal_skill(journal, "service_control");
        if !service_control_texts.is_empty() {
            if let Some(summary) =
                crate::machine_kv_projection::requested_machine_kv_summary_from_observation_inputs(
                    request_surfaces.iter().map(String::as_str),
                    &service_control_texts,
                )
            {
                return Some(summary);
            }
        }
    }
    crate::machine_kv_projection::requested_machine_kv_summary_from_observation_inputs(
        request_surfaces.iter().map(String::as_str),
        &observed_texts,
    )
}

fn observed_machine_text_fragments_from_journal_skill(
    journal: &crate::task_journal::TaskJournal,
    skill_name: &str,
) -> Vec<String> {
    let mut values = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok || step.skill != skill_name {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            output,
            &mut values,
        );
    }
    values.sort();
    values.dedup();
    values
}

fn task_machine_kv_request_surfaces(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut surfaces = Vec::new();
    for value in [
        Some(prompt),
        Some(journal.input_text.as_str()),
        Some(route_result.resolved_intent.as_str()),
        journal
            .route_result
            .as_ref()
            .map(|route| route.resolved_intent.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, value);
    }
    if let Some(state_patch) = journal
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    {
        crate::machine_kv_projection::collect_machine_kv_surfaces_from_json(
            state_patch,
            &mut surfaces,
        );
    }
    surfaces
}

fn final_answer_preserves_compare_paths_existence_fields(
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    text_has_compare_paths_existence_fields(answer_text)
        || answer_messages
            .iter()
            .any(|message| text_has_compare_paths_existence_fields(message))
}

fn text_has_compare_paths_existence_fields(text: &str) -> bool {
    let mut has_same_path = false;
    let mut has_left_exists = false;
    let mut has_right_exists = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("same_path=") {
            has_same_path = true;
        } else if line.starts_with("left_exists=") {
            has_left_exists = true;
        } else if line.starts_with("right_exists=") {
            has_right_exists = true;
        }
    }
    has_same_path && has_left_exists && has_right_exists
}

fn route_preserves_generated_file_machine_report(
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    let contract = route_result.effective_output_contract();
    if contract.delivery_required
        || !route_result
            .output_contract_marker_is(crate::OutputSemanticKind::GeneratedFilePathReport)
    {
        return false;
    }
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(generated_file_machine_report_has_multi_field_payload)
}

fn generated_file_machine_report_has_multi_field_payload(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let has_output_path = text.contains("output_path=");
    let has_planned_outputs = text.contains("planned_outputs=");
    let has_async_poll_result = text.contains("async_poll_adapter_result:");
    let has_task_status =
        text.contains("task_id:") && text.contains("job_id:") && text.contains("status:");
    (has_output_path && has_planned_outputs) || (has_async_poll_result && has_task_status)
}

#[cfg(test)]
#[path = "task_machine_kv_summary_tests.rs"]
mod task_machine_kv_summary_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
        OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
    };
    use serde_json::json;

    fn service_status_route() -> RouteResult {
        RouteResult {
            ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
            resolved_intent:
                "Check clawd service/process status and return target, status, manager_type."
                    .to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ServiceStatus,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        }
    }

    #[test]
    fn service_status_machine_kv_prefers_single_service_control_source() {
        let route = service_status_route();
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-service-kv-source",
            "ask",
            "Check clawd service/process status only; return target, status, manager_type.",
        );
        journal.record_route_result(&route);
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "process_basic",
                json!({
                    "extra": {
                        "action": "ps",
                        "filter": "clawd",
                        "match_count": 0,
                        "running": false,
                        "status": "not_running"
                    },
                    "text": "status=not_running target=clawd"
                })
                .to_string(),
            ));
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_2",
                "service_control",
                json!({
                    "extra": {
                        "status": "ok",
                        "target": "clawd",
                        "service_name": "clawd",
                        "manager_type": "rustclaw",
                        "requested_action": "status",
                        "executed_actions": ["status"],
                        "post_state": "clawd=running",
                        "verified": true
                    }
                })
                .to_string(),
            ));

        let summary = requested_machine_kv_summary_from_task_final_answer(
            "Return target, status, manager_type.",
            &route,
            &journal,
            "",
            &[],
        )
        .expect("machine summary");

        assert_eq!(summary, "target=clawd status=ok manager_type=rustclaw");
    }
}
