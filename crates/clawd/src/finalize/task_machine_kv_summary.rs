pub(super) fn recover_requested_machine_kv_summary_final_answer(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
    force_structured: bool,
) -> bool {
    if force_structured && machine_kv_recovery_blocked_by_required_content_gap(journal) {
        return false;
    }
    let applied = if force_structured {
        apply_requested_machine_kv_summary_to_final_answer_force_structured(
            prompt,
            route_result,
            journal,
            answer_text,
            answer_messages,
        )
    } else {
        apply_requested_machine_kv_summary_to_final_answer(
            prompt,
            route_result,
            journal,
            answer_text,
            answer_messages,
        )
    };
    if !applied {
        return false;
    }
    journal.record_answer_verifier_summary(crate::answer_verifier::AnswerVerifierOut {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 1.0,
    });
    true
}

fn machine_kv_recovery_blocked_by_required_content_gap(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(summary) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    summary
        .answer_incomplete_reason
        .starts_with("post_write_content_evidence_required")
        || summary
            .missing_evidence_fields
            .iter()
            .any(|field| field.trim() == "content_excerpt")
}

pub(super) fn apply_requested_machine_kv_summary_to_final_answer(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    apply_requested_machine_kv_summary_to_final_answer_inner(
        prompt,
        route_result,
        journal,
        answer_text,
        answer_messages,
        false,
    )
}

pub(super) fn apply_requested_machine_kv_summary_to_final_answer_force_structured(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    apply_requested_machine_kv_summary_to_final_answer_inner(
        prompt,
        route_result,
        journal,
        answer_text,
        answer_messages,
        true,
    )
}

fn apply_requested_machine_kv_summary_to_final_answer_inner(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
    force_structured: bool,
) -> bool {
    if final_answer_preserves_delivery_artifact(route_result, answer_text, answer_messages) {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if route_preserves_generated_file_machine_report(route_result, answer_text, answer_messages) {
        journal.record_final_answer(answer_text.as_str());
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
    if !force_structured
        && answer_verifier_passed_publishable_summary(journal, answer_text, answer_messages)
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    let request_surfaces = task_machine_kv_request_surfaces(prompt, route_result, journal);
    let requested_summary = requested_machine_kv_summary_from_task_final_answer_with_surfaces(
        &request_surfaces,
        route_result,
        journal,
        answer_text,
        answer_messages,
    );
    let requested_summary_missing_from_answer =
        requested_summary.as_deref().is_some_and(|summary| {
            answer_text.trim() != summary
                && !final_answer_preserves_structured_machine_projection(
                    answer_text,
                    answer_messages,
                    summary,
                )
                && !final_answer_has_values_for_requested_marker_summary(
                    answer_text,
                    answer_messages,
                    summary,
                )
        });
    let requested_summary_overrides_scalar_delivery = requested_summary_missing_from_answer
        && requested_summary.as_deref().is_some_and(|summary| {
            request_surfaces_explicitly_request_kv_summary(&request_surfaces, summary)
        });
    if let Some(restored) =
        web_search_candidate_listing_final_answer_from_journal(journal, answer_text)
    {
        if restored.trim() == answer_text.trim() {
            journal.record_final_answer(answer_text.as_str());
            return false;
        }
        answer_messages.clear();
        answer_messages.push(restored.clone());
        *answer_text = restored;
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
        return true;
    }
    if let Some(restored) = requested_summary
        .as_deref()
        .and_then(|summary| latest_path_batch_fact_answer_for_requested_summary(journal, summary))
    {
        if restored.trim() == answer_text.trim() {
            journal.record_final_answer(answer_text.as_str());
            return false;
        }
        answer_messages.clear();
        answer_messages.push(restored.clone());
        *answer_text = restored;
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
        return true;
    }
    if !requested_summary_overrides_scalar_delivery
        && final_answer_preserves_terminal_scalar_contract(route_result, journal, answer_text)
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if !force_structured
        && !requested_summary_overrides_scalar_delivery
        && final_answer_preserves_grounded_summary_delivery(
            route_result,
            journal,
            answer_text,
            answer_messages,
        )
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if final_answer_json_satisfies_requested_machine_tokens(
        answer_text,
        answer_messages,
        &request_surfaces,
    ) {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if let Some(restored) =
        latest_journal_json_answer_satisfies_requested_machine_tokens(journal, &request_surfaces)
    {
        if answer_text.trim() == restored.trim() {
            journal.record_final_answer(answer_text.as_str());
            return false;
        }
        answer_messages.clear();
        answer_messages.push(restored.clone());
        *answer_text = restored;
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
        return true;
    }
    let Some(summary) = requested_summary else {
        journal.record_final_answer(answer_text.as_str());
        return false;
    };
    if let Some(patched) =
        patch_current_answer_with_requested_machine_summary(answer_text, answer_messages, &summary)
    {
        if patched.trim() == answer_text.trim() {
            journal.record_final_answer(answer_text.as_str());
            return false;
        }
        answer_messages.clear();
        answer_messages.push(patched.clone());
        *answer_text = patched;
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
        return true;
    }
    if answer_text.trim() == summary {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if final_answer_preserves_structured_machine_projection(answer_text, answer_messages, &summary)
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if !force_structured
        && final_answer_preserves_publishable_evidence_summary(
            route_result,
            journal,
            answer_text,
            answer_messages,
            &summary,
        )
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if final_answer_preserves_compare_paths_existence_fields(answer_text, answer_messages)
        && !text_has_compare_paths_existence_fields(&summary)
    {
        journal.record_final_answer(answer_text.as_str());
        return false;
    }
    if !force_structured
        && final_answer_preserves_service_control_status_summary(
            route_result,
            journal,
            answer_text,
            answer_messages,
        )
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
    if !force_structured
        && final_answer_has_values_for_requested_marker_summary(
            answer_text,
            answer_messages,
            &summary,
        )
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

fn patch_current_answer_with_requested_machine_summary(
    answer_text: &str,
    answer_messages: &[String],
    requested_summary: &str,
) -> Option<String> {
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .find_map(|candidate| {
            patch_json_object_with_requested_machine_summary(candidate, requested_summary)
        })
}

fn patch_json_object_with_requested_machine_summary(
    candidate: &str,
    requested_summary: &str,
) -> Option<String> {
    let pairs = requested_machine_summary_pairs(requested_summary);
    if pairs.is_empty() {
        return None;
    }
    let mut value = serde_json::from_str::<serde_json::Value>(candidate.trim()).ok()?;
    let serde_json::Value::Object(object) = &mut value else {
        return None;
    };
    if object.is_empty() {
        return None;
    }
    let mut changed = false;
    for (key, value_text) in pairs {
        if object.get(&key).is_some_and(json_value_has_payload) {
            continue;
        }
        object.insert(key, machine_summary_value_to_json(&value_text));
        changed = true;
    }
    changed
        .then(|| serde_json::to_string(&value).ok())
        .flatten()
}

fn requested_machine_summary_pairs(requested_summary: &str) -> Vec<(String, String)> {
    machine_kv_units(requested_summary)
        .into_iter()
        .filter_map(|unit| {
            let (key, value) = unit.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn machine_summary_value_to_json(value: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(value)
        .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

fn final_answer_preserves_terminal_scalar_contract(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
) -> bool {
    if route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
    {
        return false;
    }
    let candidate = answer_text.trim();
    if !task_final_scalar_candidate_matches_route(route_result, candidate) {
        return false;
    }
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| {
            step.status == crate::executor::StepExecutionStatus::Ok && step.skill == "respond"
        })
        .filter_map(|step| step.output_excerpt.as_deref())
        .map(str::trim)
        .any(|respond| respond == candidate)
        || journal_observed_scalar_matches_candidate(journal, candidate)
}

fn task_final_scalar_candidate_matches_route(
    route_result: &crate::RouteResult,
    candidate: &str,
) -> bool {
    if candidate.is_empty()
        || candidate
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            != 1
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || candidate.contains('=')
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
    {
        return false;
    }
    if crate::finalize::route_matches_single_path_output_contract(route_result) {
        return candidate.starts_with('/')
            || candidate.starts_with("./")
            || candidate.starts_with("../")
            || candidate.contains('/');
    }
    true
}

fn journal_observed_scalar_matches_candidate(
    journal: &crate::task_journal::TaskJournal,
    candidate: &str,
) -> bool {
    journal.step_results.iter().rev().any(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        {
            return false;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            return false;
        };
        serde_json::from_str::<serde_json::Value>(output.trim())
            .ok()
            .and_then(|value| task_observed_scalar_from_json(&value))
            .is_some_and(|observed| observed.trim() == candidate)
    })
}

fn task_observed_scalar_from_json(value: &serde_json::Value) -> Option<String> {
    if let Some(answer) = value.get("extra").and_then(task_observed_scalar_from_json) {
        return Some(answer);
    }
    if let Some(value_text) = value.get("value_text").and_then(serde_json::Value::as_str) {
        let value_text = value_text.trim();
        if !value_text.is_empty() {
            return Some(value_text.to_string());
        }
        if value.get("value").and_then(serde_json::Value::as_str) == Some("") {
            return Some("\"\"".to_string());
        }
    }
    for key in ["value", "field_value", "count", "total", "schema_version"] {
        let Some(child) = value.get(key) else {
            continue;
        };
        match child {
            serde_json::Value::String(text) => {
                let text = text.trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
                if key == "value" {
                    return serde_json::to_string(text).ok();
                }
            }
            serde_json::Value::Number(number) => return Some(number.to_string()),
            serde_json::Value::Bool(value) => return Some(value.to_string()),
            _ => {}
        }
    }
    None
}

fn final_answer_preserves_delivery_artifact(
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    route_expects_delivery_artifact(route_result)
        && (!crate::extract_delivery_file_tokens(answer_text).is_empty()
            || answer_messages
                .iter()
                .any(|message| !crate::extract_delivery_file_tokens(message).is_empty()))
}

fn route_expects_delivery_artifact(route_result: &crate::RouteResult) -> bool {
    route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || crate::evidence_policy::final_answer_shape_for_route(route_result).is_some_and(|shape| {
            shape.class() == crate::evidence_policy::FinalAnswerShapeClass::DeliveryArtifact
        })
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
    _route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    let visible = std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if visible.trim().is_empty() {
        return false;
    }
    let pairs = web_search_candidate_title_sources_from_journal(journal);
    web_search_candidate_titles_are_covered(&pairs, &visible)
}

fn web_search_candidate_listing_final_answer_from_journal(
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
) -> Option<String> {
    if !text_is_machine_kv_only(answer_text) {
        return None;
    }
    let pairs = web_search_candidate_title_sources_from_journal(journal);
    web_search_candidate_listing_from_pairs(pairs)
}

fn web_search_candidate_title_sources_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || !matches!(step.skill.as_str(), "web_search_extract" | "browser_web")
        {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        for pair in web_search_candidate_title_sources_from_output(output) {
            if !pairs.iter().any(|existing| existing == &pair) {
                pairs.push(pair);
            }
        }
    }
    pairs
}

fn web_search_candidate_titles_are_covered(pairs: &[(String, String)], visible: &str) -> bool {
    let mut titles: Vec<&str> = Vec::new();
    for (title, _source) in pairs {
        let title = title.as_str();
        if !titles.contains(&title) {
            titles.push(title);
        }
    }
    !titles.is_empty() && titles.into_iter().all(|title| visible.contains(title))
}

fn web_search_candidate_listing_from_pairs(pairs: Vec<(String, String)>) -> Option<String> {
    if pairs.is_empty() {
        return None;
    }
    Some(
        pairs
            .into_iter()
            .map(|(title, source)| format!("{title} - {source}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn final_answer_preserves_service_control_status_summary(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    let contract = route_result.effective_output_contract();
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Strict
        )
        || !route_allows_model_language_delivery(route_result, contract.response_shape)
    {
        return false;
    }
    let status_values = service_control_status_observed_values(journal);
    if status_values.is_empty() {
        return false;
    }
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|candidate| candidate_matches_service_control_status(candidate, &status_values))
}

fn service_control_status_observed_values(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut values = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || step.skill != "service_control"
        {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(payload) = service_control_payload_from_output(output) else {
            continue;
        };
        for key in ["post_state", "pre_state", "summary"] {
            if let Some(value) = payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                push_service_control_status_value(&mut values, value);
            }
        }
    }
    values.sort();
    values.dedup();
    values
}

fn service_control_payload_from_output(output: &str) -> Option<serde_json::Value> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if json_value_has_service_control_status_shape(&value) {
        return Some(value);
    }
    value
        .get("extra")
        .filter(|extra| json_value_has_service_control_status_shape(extra))
        .cloned()
}

fn json_value_has_service_control_status_shape(value: &serde_json::Value) -> bool {
    (value.get("post_state").is_some()
        || value.get("pre_state").is_some()
        || value.get("summary").is_some())
        && (value.get("service_name").is_some() || value.get("target").is_some())
}

fn push_service_control_status_value(values: &mut Vec<String>, value: &str) {
    push_status_value_if_publishable(values, value);
    if let Some((_, tail)) = value.rsplit_once('=') {
        push_status_value_if_publishable(values, tail.trim());
    }
    if let Some((_, tail)) = value.rsplit_once(':') {
        push_status_value_if_publishable(values, tail.trim());
    }
}

fn push_status_value_if_publishable(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.len() < 3
        || value.eq_ignore_ascii_case("ok")
        || value.contains('\n')
        || !value.chars().any(|ch| ch.is_ascii_alphanumeric())
    {
        return;
    }
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn candidate_matches_service_control_status(candidate: &str, status_values: &[String]) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
        || text_is_json_object_or_array(candidate)
        || text_looks_like_raw_command_snapshot(candidate)
        || text_is_machine_kv_only(candidate)
    {
        return false;
    }
    status_values
        .iter()
        .any(|value| candidate_has_observed_status_value(candidate, value))
}

fn candidate_has_observed_status_value(candidate: &str, observed: &str) -> bool {
    if candidate.contains(observed) {
        return true;
    }
    if !observed.is_ascii() {
        return false;
    }
    candidate
        .to_ascii_lowercase()
        .contains(observed.to_ascii_lowercase().as_str())
}

fn route_is_weather_query(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_namespace(route, &["weather"])
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

fn final_answer_json_satisfies_requested_machine_tokens(
    answer_text: &str,
    answer_messages: &[String],
    request_surfaces: &[String],
) -> bool {
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|candidate| {
            json_object_satisfies_requested_machine_tokens(candidate, request_surfaces)
        })
}

fn latest_journal_json_answer_satisfies_requested_machine_tokens(
    journal: &crate::task_journal::TaskJournal,
    request_surfaces: &[String],
) -> Option<String> {
    for step in journal.step_results.iter().rev() {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        {
            continue;
        }
        if let Some(candidate) = step.output_excerpt.as_deref().and_then(|candidate| {
            json_answer_satisfies_requested_machine_tokens(candidate, request_surfaces)
        }) {
            return Some(candidate);
        }
    }
    None
}

fn json_answer_satisfies_requested_machine_tokens(
    candidate: &str,
    request_surfaces: &[String],
) -> Option<String> {
    let candidate = candidate.trim();
    if json_object_satisfies_requested_machine_tokens(candidate, request_surfaces) {
        return Some(candidate.to_string());
    }
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(candidate)
    else {
        return None;
    };
    let answer = object
        .get("answer")
        .and_then(serde_json::Value::as_str)?
        .trim();
    json_object_satisfies_requested_machine_tokens(answer, request_surfaces)
        .then(|| answer.to_string())
}

fn json_object_satisfies_requested_machine_tokens(
    candidate: &str,
    request_surfaces: &[String],
) -> bool {
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(candidate.trim())
    else {
        return false;
    };
    !object.is_empty()
        && object.len() <= 16
        && object.iter().all(|(key, value)| {
            valid_machine_marker_key(key)
                && json_value_has_payload(value)
                && request_surfaces
                    .iter()
                    .any(|surface| surface_contains_machine_token(surface, key))
        })
}

fn json_value_has_payload(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(object) => !object.is_empty(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

fn surface_contains_machine_token(surface: &str, token: &str) -> bool {
    !token.is_empty()
        && surface
            .split(|ch| !machine_token_char(ch))
            .any(|segment| segment == token)
}

fn machine_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

fn final_answer_preserves_publishable_evidence_summary(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
    requested_summary: &str,
) -> bool {
    let contract = route_result.effective_output_contract();
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !route_allows_model_language_delivery(route_result, contract.response_shape)
        || !journal_has_observed_tool_evidence(journal)
        || machine_kv_units(requested_summary).is_empty()
    {
        return false;
    }
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|candidate| candidate_is_publishable_evidence_summary(candidate, requested_summary))
}

fn answer_verifier_passed_publishable_summary(
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    let Some(verifier) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !verifier.pass
        || verifier.should_retry
        || !verifier.missing_evidence_fields.is_empty()
        || !journal_has_observed_tool_evidence(journal)
    {
        return false;
    }
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|candidate| candidate_is_publishable_evidence_summary(candidate, ""))
}

fn journal_has_observed_tool_evidence(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
            && step.output_excerpt.as_deref().is_some_and(|output| {
                !output.trim().is_empty()
                    && !crate::finalize::looks_like_planner_artifact(output)
                    && !crate::finalize::looks_like_internal_trace_artifact(output)
            })
    })
}

fn route_allows_model_language_delivery(
    route_result: &crate::RouteResult,
    response_shape: crate::OutputResponseShape,
) -> bool {
    crate::evidence_policy::final_answer_shape_for_route(route_result)
        .is_some_and(|shape| shape.allows_model_language())
        || matches!(
            response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
}

fn final_answer_preserves_grounded_summary_delivery(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    let Some(shape) = crate::evidence_policy::final_answer_shape_for_route(route_result) else {
        return false;
    };
    let contract = route_result.effective_output_contract();
    if shape.class() != crate::evidence_policy::FinalAnswerShapeClass::GroundedSummary
        || contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || !shape.allows_model_language()
        || !journal_has_observed_tool_evidence(journal)
    {
        return false;
    }
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|candidate| candidate_is_publishable_evidence_summary(candidate, ""))
}

fn candidate_is_publishable_evidence_summary(candidate: &str, requested_summary: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
        || text_is_json_object_or_array(candidate)
        || text_looks_like_raw_command_snapshot(candidate)
        || text_is_machine_kv_only(candidate)
    {
        return false;
    }
    let candidate_chars = candidate.chars().count();
    let summary_chars = requested_summary.trim().chars().count();
    let nonempty_lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let token_count = candidate.split_whitespace().count();
    candidate_chars > summary_chars.saturating_add(16)
        && (nonempty_lines > 1 || token_count >= 6 || candidate_chars >= 48)
}

fn text_looks_like_raw_command_snapshot(text: &str) -> bool {
    let text = text.trim();
    text.starts_with("exit=")
        && text.contains('\n')
        && (text.contains("\nCOMMAND ")
            || text.contains("(LISTEN)")
            || text.contains("\nLISTEN ")
            || text.contains("State  Recv-Q")
            || text.contains("%CPU")
            || text.contains("PID PPID"))
}

fn text_is_machine_kv_only(text: &str) -> bool {
    let mut saw_line = false;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        saw_line = true;
        let units = machine_kv_units(line);
        if units.is_empty() || units.join(" ") != line {
            return false;
        }
    }
    saw_line
}

fn final_answer_preserves_structured_machine_projection(
    answer_text: &str,
    answer_messages: &[String],
    requested_summary: &str,
) -> bool {
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|candidate| {
            let candidate = candidate.trim();
            text_is_structured_machine_field_projection(candidate)
                && machine_projection_covers_requested_summary(candidate, requested_summary)
        })
}

fn text_is_structured_machine_field_projection(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let mut anchored = false;
    for line in &lines {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim();
        let value = value.trim();
        if !valid_machine_marker_key(key) || value.is_empty() {
            return false;
        }
        if key.contains('.')
            || value.starts_with('{')
            || value.starts_with('[')
            || matches!(
                key,
                "async_cancel_adapter_result"
                    | "async_poll_adapter_result"
                    | "dry_run"
                    | "job_id"
                    | "model"
                    | "model_kind"
                    | "output_path"
                    | "planned_outputs"
                    | "provider"
                    | "status"
                    | "task_id"
            )
        {
            anchored = true;
        }
    }
    anchored
}

fn machine_projection_covers_requested_summary(candidate: &str, requested_summary: &str) -> bool {
    let requested_units = machine_kv_units(requested_summary);
    if requested_units.is_empty() {
        return true;
    }
    let current_units = machine_kv_units(candidate);
    !current_units.is_empty()
        && current_units.len() >= requested_units.len()
        && requested_units
            .iter()
            .all(|unit| current_units.iter().any(|current| current == unit))
}

fn machine_kv_units(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            });
            let (key, value) = token.split_once('=')?;
            if valid_machine_marker_key(key) && !value.trim().is_empty() {
                Some(format!("{}={}", key.trim(), value.trim()))
            } else {
                None
            }
        })
        .collect()
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

fn latest_path_batch_fact_answer_for_requested_summary(
    journal: &crate::task_journal::TaskJournal,
    requested_summary: &str,
) -> Option<String> {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .find_map(|output| {
            let fact = path_batch_fact_answer_from_output(output)?;
            requested_summary_refs_path_fact(requested_summary, &fact.path)
                .then(|| fact.into_machine_answer())
        })
}

struct PathBatchFactAnswer {
    path: String,
    exists: bool,
    kind: Option<String>,
    size_bytes: Option<u64>,
}

impl PathBatchFactAnswer {
    fn into_machine_answer(self) -> String {
        let mut lines = vec![
            "message_key=clawd.msg.path_fact.observed".to_string(),
            "reason_code=path_fact_observed".to_string(),
            format!("exists={}", self.exists),
            format!("path={}", sanitize_machine_line_value(&self.path)),
        ];
        let kind = self
            .kind
            .filter(|kind| !kind.is_empty())
            .or_else(|| (!self.exists).then(|| "missing".to_string()));
        if let Some(kind) = kind {
            lines.push(format!("kind={kind}"));
        }
        if let Some(size_bytes) = self.size_bytes {
            lines.push(format!("size_bytes={size_bytes}"));
        }
        lines.push("source_action=path_batch_facts".to_string());
        lines.join("\n")
    }
}

fn path_batch_fact_answer_from_output(output: &str) -> Option<PathBatchFactAnswer> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    let value = path_batch_facts_payload(&value)?;
    let facts = value.get("facts").and_then(serde_json::Value::as_array)?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    let exists = entry
        .get("exists")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let fact = entry.get("fact").and_then(serde_json::Value::as_object);
    let path = fact
        .and_then(|fact| fact.get("resolved_path"))
        .or_else(|| fact.and_then(|fact| fact.get("path")))
        .or_else(|| entry.get("resolved_path"))
        .or_else(|| entry.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?
        .to_string();
    let kind = fact
        .and_then(|fact| fact.get("kind"))
        .or_else(|| entry.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(normalized_path_kind_token);
    let size_bytes = fact
        .and_then(|fact| fact.get("size_bytes"))
        .or_else(|| entry.get("size_bytes"))
        .and_then(serde_json::Value::as_u64);
    Some(PathBatchFactAnswer {
        path,
        exists,
        kind,
        size_bytes,
    })
}

fn path_batch_facts_payload(value: &serde_json::Value) -> Option<&serde_json::Value> {
    if value.get("action").and_then(serde_json::Value::as_str) == Some("path_batch_facts") {
        return Some(value);
    }
    let extra = value.get("extra")?;
    (extra.get("action").and_then(serde_json::Value::as_str) == Some("path_batch_facts"))
        .then_some(extra)
}

fn requested_summary_refs_path_fact(requested_summary: &str, path: &str) -> bool {
    let requested_summary = requested_summary.trim();
    if requested_summary.is_empty() {
        return false;
    }
    bare_machine_markers(requested_summary)
        .iter()
        .any(|marker| path_matches_requested_token(path, marker))
        || requested_machine_summary_pairs(requested_summary)
            .iter()
            .any(|(key, value)| {
                matches!(key.as_str(), "path" | "resolved_path")
                    && path_matches_requested_token(path, value)
            })
}

fn path_matches_requested_token(path: &str, token: &str) -> bool {
    let token = token.trim().trim_matches('`');
    let path = path.trim();
    !token.is_empty()
        && !path.is_empty()
        && (path == token
            || path.rsplit('/').next().is_some_and(|name| name == token)
            || path.ends_with(&format!("/{token}")))
}

fn normalized_path_kind_token(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "dir" | "directory" => "dir".to_string(),
        "file" => "file".to_string(),
        "symlink" | "link" => "symlink".to_string(),
        "missing" | "not_found" | "not found" => "missing".to_string(),
        "other" => "other".to_string(),
        "unknown" => "unknown".to_string(),
        value => value.to_string(),
    }
}

fn sanitize_machine_line_value(value: &str) -> String {
    crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    )
}

#[cfg(test)]
fn requested_machine_kv_summary_from_task_final_answer(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    let request_surfaces = task_machine_kv_request_surfaces(prompt, route_result, journal);
    requested_machine_kv_summary_from_task_final_answer_with_surfaces(
        &request_surfaces,
        route_result,
        journal,
        answer_text,
        answer_messages,
    )
}

fn requested_machine_kv_summary_from_task_final_answer_with_surfaces(
    request_surfaces: &[String],
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
    if route_requests_service_status_machine_kv_summary(route_result) {
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

fn request_surfaces_explicitly_request_kv_summary(
    request_surfaces: &[String],
    requested_summary: &str,
) -> bool {
    let summary_units = machine_kv_units(requested_summary);
    !summary_units.is_empty()
        && request_surfaces.iter().any(|surface| {
            let surface_units = machine_kv_units(surface);
            !surface_units.is_empty()
                && summary_units.iter().any(|unit| {
                    surface_units
                        .iter()
                        .any(|surface_unit| surface_unit == unit)
                })
        })
}

fn route_requests_service_status_machine_kv_summary(route: &crate::RouteResult) -> bool {
    crate::finalize::route_matches_service_control_machine_summary(route)
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
        crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
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
    let has_async_poll_result =
        text.contains("async_poll_adapter_result:") || text.contains("async_poll_adapter_result=");
    let has_async_cancel_result = text.contains("async_cancel_adapter_result:")
        || text.contains("async_cancel_adapter_result=");
    let has_task_status = (text.contains("task_id:") || text.contains("task_id="))
        && (text.contains("job_id:") || text.contains("job_id="))
        && (text.contains("status:") || text.contains("status="));
    (has_output_path && has_planned_outputs)
        || ((has_async_poll_result || has_async_cancel_result) && has_task_status)
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
            ask_mode: crate::AskMode::act_with_chat_finalizer(),
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

    #[test]
    fn service_capability_ref_machine_kv_prefers_single_service_control_source_without_semantic_kind(
    ) {
        let mut route = service_status_route();
        route.route_reason = "capability_ref=service.status".to_string();
        route.output_contract.apply_output_contract_ref(
            crate::pipeline_types::OutputContractRef::new(OutputSemanticKind::None),
        );
        let requested_fields = ["target", "status", "manager_type"].join(" ");
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-service-kv-capability-source",
            "ask",
            &requested_fields,
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
                    }
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
            &requested_fields,
            &route,
            &journal,
            "",
            &[],
        )
        .expect("machine summary");

        assert_eq!(summary, "target=clawd status=ok manager_type=rustclaw");
    }
}
