use super::*;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TaskJournalEvidenceCoverage {
    pub(crate) required_evidence: Vec<String>,
    pub(crate) evidence_expression: Option<Value>,
    pub(crate) observed_fields: BTreeSet<String>,
    pub(crate) observed_canonical: BTreeSet<String>,
    pub(crate) observed_extractors: BTreeSet<String>,
    pub(crate) source_refs: BTreeSet<String>,
    pub(crate) observed_evidence_sources: BTreeMap<String, BTreeSet<String>>,
    pub(crate) missing_evidence: Vec<String>,
    pub(crate) confidence: f64,
    pub(crate) repair_eligible: bool,
}

impl TaskJournalEvidenceCoverage {
    pub(crate) fn is_complete(&self) -> bool {
        self.missing_evidence.is_empty()
    }

    pub(super) fn to_trace_json(&self) -> Value {
        json!({
            "schema_version": 1,
            "required_evidence": self.required_evidence.clone(),
            "evidence_expression": self.evidence_expression.clone(),
            "observed_fields": self.observed_fields.iter().take(64).cloned().collect::<Vec<_>>(),
            "observed_canonical": self.observed_canonical.iter().take(64).cloned().collect::<Vec<_>>(),
            "observed_extractors": self.observed_extractors.iter().take(64).cloned().collect::<Vec<_>>(),
            "source_refs": self.source_refs.iter().take(64).cloned().collect::<Vec<_>>(),
            "observed_evidence_sources": observed_evidence_sources_trace_json(&self.observed_evidence_sources),
            "missing_evidence": self.missing_evidence.clone(),
            "confidence": self.confidence,
            "repair_eligible": self.repair_eligible,
        })
    }
}

pub(crate) fn evidence_coverage_for_output_contract(
    output_contract: &crate::IntentOutputContract,
    journal: &TaskJournal,
) -> TaskJournalEvidenceCoverage {
    let action_override = successful_action_required_evidence_override(journal);
    let required_evidence = action_override.clone().unwrap_or_else(|| {
        crate::evidence_policy::required_evidence_fields_for_output_contract(output_contract)
    });
    let (observed_fields, mut observed_canonical, observed_extractors, observed_evidence_sources) =
        observed_evidence_field_sets(journal);
    augment_output_contract_canonical_evidence(
        output_contract,
        &required_evidence,
        &observed_fields,
        &observed_extractors,
        &mut observed_canonical,
    );
    let evidence_expression = if action_override.is_some() {
        None
    } else {
        crate::evidence_policy::evidence_expression_for_output_contract(output_contract)
    };
    let missing_evidence = evidence_expression
        .as_ref()
        .filter(|expression| evidence_expression_has_requirements(expression))
        .map(|expression| missing_evidence_for_expression(expression, &observed_canonical))
        .unwrap_or_else(|| missing_required_evidence(&required_evidence, &observed_canonical));
    let confidence = evidence_coverage_confidence(&required_evidence, &missing_evidence);
    let repair_eligible = evidence_coverage_repair_eligible(output_contract, &missing_evidence);
    let source_refs = source_refs_from_observed_sources(&observed_evidence_sources);
    TaskJournalEvidenceCoverage {
        required_evidence,
        evidence_expression: evidence_expression
            .as_ref()
            .map(|expression| expression.to_trace_json(&[])),
        observed_fields,
        observed_canonical,
        observed_extractors,
        source_refs,
        observed_evidence_sources,
        missing_evidence,
        confidence,
        repair_eligible,
    }
}

fn successful_action_required_evidence_override(journal: &TaskJournal) -> Option<Vec<String>> {
    let successful_step_ids = journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .map(|step| step.step_id.as_str())
        .collect::<BTreeSet<_>>();
    if successful_step_ids.is_empty() {
        return None;
    }
    journal_planned_steps(journal)
        .into_iter()
        .filter(|step| successful_step_ids.contains(step.step_id.as_str()))
        .find_map(planned_step_required_evidence_override)
}

fn journal_planned_steps(journal: &TaskJournal) -> Vec<&crate::PlanStep> {
    let mut steps = Vec::new();
    if let Some(plan) = journal.plan_result.as_ref() {
        steps.extend(plan.steps.iter());
    }
    for round in &journal.rounds {
        if let Some(plan) = round.plan_result.as_ref() {
            steps.extend(plan.steps.iter());
        }
    }
    steps
}

fn planned_step_required_evidence_override(step: &crate::PlanStep) -> Option<Vec<String>> {
    let skill = step.skill.trim();
    let action = step
        .args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("");
    if matches!(step.action_type.as_str(), "call_tool" | "call_skill")
        && matches!(skill, "db_basic" | "database" | "db" | "sqlite")
        && action == "schema_version"
    {
        return Some(vec!["field_value".to_string()]);
    }
    if matches!(step.action_type.as_str(), "call_tool" | "call_skill")
        && matches!(skill, "system_basic" | "system")
        && action == "runtime_status"
    {
        return Some(vec!["field_value".to_string()]);
    }
    if step.action_type == "call_capability"
        && capability_value_has_action_name(skill, &["database", "db", "sqlite"], "schema_version")
    {
        return Some(vec!["field_value".to_string()]);
    }
    if step.action_type == "call_capability"
        && capability_value_has_action_name(skill, &["system", "system_basic"], "runtime_status")
    {
        return Some(vec!["field_value".to_string()]);
    }
    None
}

fn capability_value_has_action_name(value: &str, namespaces: &[&str], action: &str) -> bool {
    let Some((namespace, candidate_action)) = value.trim().split_once('.') else {
        return false;
    };
    namespaces
        .iter()
        .any(|candidate| namespace == candidate.trim())
        && candidate_action == action
}

pub(super) fn source_refs_from_observed_sources(
    observed_evidence_sources: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    observed_evidence_sources
        .values()
        .flat_map(|extractors| extractors.iter().cloned())
        .collect()
}

pub(super) fn evidence_coverage_confidence(
    required_evidence: &[String],
    missing_evidence: &[String],
) -> f64 {
    if required_evidence.is_empty() && missing_evidence.is_empty() {
        return 1.0;
    }
    let required_count = required_evidence.len().max(missing_evidence.len());
    if required_count == 0 {
        return 1.0;
    }
    let present_count = required_count.saturating_sub(missing_evidence.len());
    present_count as f64 / required_count as f64
}

pub(super) fn evidence_coverage_repair_eligible(
    output_contract: &crate::IntentOutputContract,
    missing_evidence: &[String],
) -> bool {
    !missing_evidence.is_empty() && !output_contract.delivery_required
}

pub(super) fn observed_evidence_sources_trace_json(
    observed_evidence_sources: &BTreeMap<String, BTreeSet<String>>,
) -> Value {
    Value::Object(
        observed_evidence_sources
            .iter()
            .take(64)
            .map(|(field, extractors)| {
                (
                    field.clone(),
                    json!(extractors.iter().take(16).cloned().collect::<Vec<_>>()),
                )
            })
            .collect(),
    )
}

pub(super) fn missing_required_evidence(
    required_evidence: &[String],
    observed_canonical: &BTreeSet<String>,
) -> Vec<String> {
    required_evidence
        .iter()
        .filter(|field| !observed_canonical.contains(field.as_str()))
        .cloned()
        .collect()
}

pub(super) fn missing_evidence_for_expression(
    expression: &crate::evidence_policy::EvidenceExpression,
    observed_canonical: &BTreeSet<String>,
) -> Vec<String> {
    let mut missing = Vec::new();
    missing.extend(
        expression
            .all_of
            .iter()
            .filter(|field| !observed_canonical.contains(field.as_str()))
            .cloned(),
    );
    if !expression.one_of.is_empty()
        && !expression
            .one_of
            .iter()
            .any(|field| observed_canonical.contains(field.as_str()))
    {
        missing.push(format!("one_of({})", expression.one_of.join("|")));
    }
    if !expression.any_of.is_empty()
        && !expression
            .any_of
            .iter()
            .any(|field| observed_canonical.contains(field.as_str()))
    {
        missing.push(format!("any_of({})", expression.any_of.join("|")));
    }
    missing.extend(
        expression
            .negative_evidence
            .iter()
            .filter(|field| {
                observed_canonical.contains(field.as_str())
                    && !expression.one_of.iter().any(|allowed| allowed == *field)
            })
            .map(|field| format!("negative_evidence({field})")),
    );
    missing.dedup();
    missing
}

fn evidence_expression_has_requirements(
    expression: &crate::evidence_policy::EvidenceExpression,
) -> bool {
    !expression.all_of.is_empty()
        || !expression.one_of.is_empty()
        || !expression.any_of.is_empty()
        || !expression.negative_evidence.is_empty()
}

pub(super) fn evidence_coverage_trace_json(
    output_contract: &crate::IntentOutputContract,
    journal: &TaskJournal,
) -> Value {
    evidence_coverage_for_output_contract(output_contract, journal).to_trace_json()
}

pub(super) fn task_outcome_summary_json(journal: &TaskJournal) -> Value {
    let final_shape = journal
        .output_contract
        .as_ref()
        .and_then(crate::evidence_policy::trace_snapshot_for_output_contract)
        .and_then(|snapshot| {
            snapshot
                .get("final_answer_shape")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
    let missing_evidence = journal
        .output_contract
        .as_ref()
        .map(|contract| evidence_coverage_for_output_contract(contract, journal).missing_evidence)
        .unwrap_or_default();
    let missing_count = missing_evidence.len();
    let state = match journal.final_status {
        Some(TaskJournalFinalStatus::Success) if missing_count == 0 => "completed",
        Some(TaskJournalFinalStatus::Success) => "needs_attention",
        Some(TaskJournalFinalStatus::Clarify) => "needs_input",
        Some(TaskJournalFinalStatus::Failure | TaskJournalFinalStatus::ResumeFailure) => "failed",
        None => "in_progress",
    };
    let (message_key, next_action_kind) = match state {
        "completed" => ("clawd.task_outcome.completed", "review_result"),
        "needs_attention" => (
            "clawd.task_outcome.needs_attention",
            "inspect_missing_evidence",
        ),
        "needs_input" => ("clawd.task_outcome.needs_input", "provide_user_input"),
        "failed" if missing_count > 0 => (
            "clawd.task_outcome.failed_missing_evidence",
            "provide_clearer_target",
        ),
        "failed" => ("clawd.task_outcome.failed", "inspect_error"),
        _ => ("clawd.task_outcome.in_progress", "poll_task"),
    };
    json!({
        "schema_version": 1,
        "state": state,
        "message_key": message_key,
        "next_action_kind": next_action_kind,
        "render_owner": "finalizer_or_ui_i18n",
        "requires_user_input": state == "needs_input",
        "terminal": matches!(state, "completed" | "needs_attention" | "needs_input" | "failed"),
        "final_answer_shape": final_shape,
        "missing_evidence_count": missing_count,
        "missing_evidence": missing_evidence,
        "has_technical_details": true,
    })
}

pub(super) fn observed_evidence_field_sets(
    journal: &TaskJournal,
) -> (
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeMap<String, BTreeSet<String>>,
) {
    let mut observed_fields = BTreeSet::new();
    let mut observed_canonical = BTreeSet::new();
    let mut observed_extractors = BTreeSet::new();
    let mut observed_evidence_sources = BTreeMap::<String, BTreeSet<String>>::new();
    for step in &journal.step_results {
        if !step_can_supply_contract_evidence(step, journal.output_contract.as_ref()) {
            continue;
        }
        let Some(evidence) = observed_evidence_for_step_trace(step) else {
            continue;
        };
        ingest_observed_evidence_value(
            &evidence,
            &mut observed_fields,
            &mut observed_canonical,
            &mut observed_extractors,
            &mut observed_evidence_sources,
        );
    }
    for observation in &journal.task_observations {
        let Some(evidence) = observation.get("observed_evidence") else {
            continue;
        };
        ingest_observed_evidence_value(
            evidence,
            &mut observed_fields,
            &mut observed_canonical,
            &mut observed_extractors,
            &mut observed_evidence_sources,
        );
    }
    (
        observed_fields,
        observed_canonical,
        observed_extractors,
        observed_evidence_sources,
    )
}

fn ingest_observed_evidence_value(
    evidence: &Value,
    observed_fields: &mut BTreeSet<String>,
    observed_canonical: &mut BTreeSet<String>,
    observed_extractors: &mut BTreeSet<String>,
    observed_evidence_sources: &mut BTreeMap<String, BTreeSet<String>>,
) {
    let extractor_ref = evidence
        .pointer("/extractor/extractor_ref")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(extractor_ref) = extractor_ref.as_ref() {
        observed_extractors.insert(extractor_ref.clone());
    }
    let Some(items) = evidence.get("items").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let Some(field) = item.get("field").and_then(Value::as_str) else {
            continue;
        };
        let normalized = normalize_evidence_field(field);
        if normalized.is_empty() {
            continue;
        }
        observed_fields.insert(normalized.clone());
        let canonical_fields = canonical_evidence_fields_for_observed_item(&normalized, item);
        if let Some(extractor_ref) = extractor_ref.as_ref() {
            observed_evidence_sources
                .entry(normalized.clone())
                .or_default()
                .insert(extractor_ref.clone());
        }
        for canonical in canonical_fields {
            if let Some(extractor_ref) = extractor_ref.as_ref() {
                observed_evidence_sources
                    .entry(canonical.clone())
                    .or_default()
                    .insert(extractor_ref.clone());
            }
            observed_canonical.insert(canonical);
        }
    }
}

pub(super) fn augment_output_contract_canonical_evidence(
    output_contract: &crate::IntentOutputContract,
    required_evidence: &[String],
    observed_fields: &BTreeSet<String>,
    observed_extractors: &BTreeSet<String>,
    observed_canonical: &mut BTreeSet<String>,
) {
    if let Some(fields) = output_contract
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(crate::machine_kv_projection::exact_machine_field_selector)
    {
        for field in fields {
            if observed_field_present(observed_fields, &field) {
                observed_canonical.insert(field);
            }
        }
    }
    for required in required_evidence {
        if observed_extractors
            .iter()
            .any(|extractor_ref| explicit_text_extractor_provides(extractor_ref, required))
        {
            observed_canonical.insert(required.clone());
        }
    }
    if output_contract.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && (observed_canonical.contains("content_excerpt")
            || observed_canonical.contains("field_value")
            || observed_fields.contains("excerpt")
            || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("command_output".to_string());
    }
    if output_contract.semantic_kind_is(crate::OutputSemanticKind::ScalarCount)
        && (observed_canonical.contains("value") || observed_canonical.contains("field_value"))
    {
        observed_canonical.insert("count".to_string());
    }
    if output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && output_contract.requires_content_evidence
        && current_workspace_inventory_fields_present(observed_fields, observed_canonical)
    {
        observed_canonical.insert("field_value".to_string());
    }
    if output_contract.semantic_kind_is_any(&[
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
    ]) && http_response_body_fields_present(observed_fields)
    {
        observed_canonical.insert("content_excerpt".to_string());
    }
}

pub(super) fn observed_field_with_prefix(observed_fields: &BTreeSet<String>, prefix: &str) -> bool {
    observed_fields
        .iter()
        .any(|field| field.starts_with(prefix))
}

fn current_workspace_inventory_fields_present(
    observed_fields: &BTreeSet<String>,
    observed_canonical: &BTreeSet<String>,
) -> bool {
    (observed_canonical.contains("candidates") || observed_canonical.contains("count"))
        && (observed_field_present(observed_fields, "names_by_kind")
            || observed_field_with_prefix(observed_fields, "names_by_kind.")
            || observed_field_with_prefix(observed_fields, "extra.names_by_kind.")
            || observed_field_present(observed_fields, "counts")
            || observed_field_with_prefix(observed_fields, "counts.")
            || observed_field_with_prefix(observed_fields, "extra.counts."))
}

pub(super) fn http_response_body_fields_present(observed_fields: &BTreeSet<String>) -> bool {
    observed_fields.contains("body") || observed_field_with_prefix(observed_fields, "body.")
}

pub(super) fn normalized_field_leaf(field: &str) -> &str {
    let leaf = field.rsplit('.').next().unwrap_or(field);
    leaf.split_once('[').map_or(leaf, |(prefix, _)| prefix)
}

pub(super) fn step_can_supply_contract_evidence(
    step: &TaskJournalStepTrace,
    output_contract: Option<&crate::IntentOutputContract>,
) -> bool {
    if matches!(
        step.skill.as_str(),
        "respond" | "synthesize_answer" | "think" | "answer_verifier"
    ) {
        return false;
    }
    if let Some(output_contract) = output_contract {
        if !output_contract.requires_content_evidence
            && !output_contract.delivery_required
            && step_reads_text_content(step)
        {
            return false;
        }
    }
    match step.status {
        crate::executor::StepExecutionStatus::Ok => true,
        crate::executor::StepExecutionStatus::Error => {
            step_error_supplies_negative_contract_evidence(step, output_contract)
                || step.skill == "run_cmd"
                    && output_contract.is_some_and(|contract| {
                        contract.semantic_kind_is(crate::OutputSemanticKind::ExecutionFailedStep)
                            || crate::evidence_policy::required_evidence_fields_for_output_contract(
                                contract,
                            )
                            .iter()
                            .any(|field| field == "command_output")
                    })
        }
    }
}

pub(super) fn step_error_supplies_negative_contract_evidence(
    step: &TaskJournalStepTrace,
    output_contract: Option<&crate::IntentOutputContract>,
) -> bool {
    let Some(output_contract) = output_contract else {
        return false;
    };
    if !output_contract.semantic_kind_is_any(&[
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
        crate::OutputSemanticKind::ExistenceWithPath,
    ]) {
        return false;
    }
    let Some(error) = step
        .error_excerpt
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    else {
        return false;
    };
    if error.starts_with("__RC_READ_FILE_NOT_FOUND__:") {
        return true;
    }
    crate::skills::parse_structured_skill_error(error)
        .is_some_and(|structured| structured.error_kind == "not_found")
}

pub(crate) fn step_reads_text_content(step: &TaskJournalStepTrace) -> bool {
    match step.skill.as_str() {
        "read_file" | "doc_parse" => return true,
        _ => {}
    }
    let Some(output) = step.output_excerpt.as_deref().map(str::trim) else {
        return false;
    };
    if output.is_empty() {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return false;
    };
    let action = step_output_action_value(&value).map(normalize_evidence_field);
    if matches!(
        action.as_deref(),
        Some("read_range" | "read_text_range" | "read_file")
    ) {
        return true;
    }
    action.as_deref() == Some("read")
        && [
            "/content",
            "/content_excerpt",
            "/excerpt",
            "/extra/content",
            "/extra/content_excerpt",
            "/extra/excerpt",
        ]
        .iter()
        .any(|pointer| {
            value
                .pointer(pointer)
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty())
        })
}

fn step_output_action_value(value: &Value) -> Option<&str> {
    value
        .get("action")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/extra/action").and_then(Value::as_str))
}

pub(super) fn observed_field_present(observed_fields: &BTreeSet<String>, field: &str) -> bool {
    observed_fields.contains(field) || observed_fields.contains(&format!("extra.{field}"))
}

pub(super) fn normalize_evidence_field(field: &str) -> String {
    field
        .trim()
        .trim_matches('.')
        .to_ascii_lowercase()
        .replace('-', "_")
}

pub(super) fn canonical_evidence_fields_for_observed_field(field: &str) -> Vec<String> {
    let leaf = normalized_field_leaf(field);
    let mut values = BTreeSet::new();
    values.insert(field.to_string());
    values.insert(leaf.to_string());

    for (canonical, aliases) in [
        (
            "candidates",
            &[
                "candidates",
                "items",
                "names",
                "paths",
                "files",
                "entries",
                "results",
                "matches",
                "name_results",
                "facts",
                "rows",
                "tables",
                "containers",
                "images",
                "members",
                "risks",
                "children",
            ][..],
        ),
        (
            "content_excerpt",
            &[
                "content_excerpt",
                "excerpt",
                "body",
                "content",
                "text",
                "lines",
                "text_excerpt",
                "recent_matches",
                "recent_notable_lines",
                "tail_lines",
                "tail_excerpt",
            ][..],
        ),
        (
            "path",
            &[
                "path",
                "resolved_path",
                "file_path",
                "db_path",
                "database_path",
                "target_path",
                "output_path",
                "archive",
                "archive_path",
                "dest",
                "dest_path",
                "destination",
                "cwd",
                "workspace_root",
                "ports",
                "public_ports",
                "listeners",
                "public_listeners",
            ][..],
        ),
        (
            "field_value",
            &[
                "field_value",
                "new_value",
                "old_value",
                "value_text",
                "value",
                "status",
                "state",
                "version",
                "schema_version",
                "package_manager",
                "manager",
                "subject",
                "branch",
                "commit",
                "valid",
                "validated",
                "available",
                "healthy",
                "running",
                "is_running",
                "port_open",
                "process_count",
                "clawd_health_port_open",
                "clawd_process_count",
                "modified_ts",
                "modified",
                "mtime",
                "mtime_ts",
                "exit",
                "exit_code",
                "error_kind",
                "keyword_counts",
                "level_counts",
                "hostname",
                "os",
                "arch",
                "cwd",
                "workspace_root",
            ][..],
        ),
        (
            "count",
            &[
                "count",
                "total",
                "length",
                "item_count",
                "row_count",
                "risk_count",
                "listener_count",
                "public_listener_count",
                "localhost_listener_count",
            ][..],
        ),
        (
            "size_bytes",
            &[
                "size_bytes",
                "total_size_bytes",
                "bytes",
                "file_size",
                "size",
            ][..],
        ),
        ("exists", &["exists", "found", "present"][..]),
        ("kind", &["kind", "file_type", "type"][..]),
        (
            "command_output",
            &[
                "command_output",
                "stdout",
                "stderr",
                "output",
                "text_excerpt",
            ][..],
        ),
    ] {
        if aliases
            .iter()
            .any(|alias| *alias == leaf || *alias == field)
        {
            values.insert(canonical.to_string());
        }
    }
    values.into_iter().collect()
}

pub(super) fn canonical_evidence_fields_for_observed_item(
    field: &str,
    item: &Value,
) -> Vec<String> {
    let mut values = canonical_evidence_fields_for_observed_field(field)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if let Some(keys) = item.get("keys").and_then(Value::as_array) {
        for key in keys.iter().filter_map(Value::as_str) {
            values.extend(canonical_evidence_fields_for_observed_field(
                &normalize_evidence_field(key),
            ));
        }
    }
    if let Some(sample_keys) = item.get("sample_keys").and_then(Value::as_array) {
        for key in sample_keys.iter().filter_map(Value::as_str) {
            values.extend(canonical_evidence_fields_for_observed_field(
                &normalize_evidence_field(key),
            ));
        }
    }
    if values.contains("exists")
        && item.get("kind").and_then(Value::as_str) == Some("bool")
        && item.get("excerpt").and_then(Value::as_str).is_some()
    {
        match item
            .get("excerpt")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "true" => {
                values.insert("exists_true".to_string());
            }
            "false" => {
                values.insert("exists_false".to_string());
            }
            _ => {}
        }
    }
    if matches!(
        field
            .split_once('[')
            .map(|(prefix, _)| normalized_field_leaf(prefix)),
        Some("results" | "name_results" | "paths" | "candidates" | "entries")
    ) && item
        .get("excerpt")
        .and_then(Value::as_str)
        .is_some_and(text_line_looks_like_path)
    {
        values.insert("path".to_string());
    }
    values.into_iter().collect()
}

pub(super) fn structured_error_extra_string(
    structured_error: Option<&crate::skills::StructuredSkillError>,
    key: &str,
) -> Option<String> {
    structured_error
        .and_then(|value| value.extra.as_ref())
        .and_then(|extra| extra.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn structured_error_failure_attribution(
    structured_error: Option<&crate::skills::StructuredSkillError>,
) -> Option<String> {
    if let Some(raw) = structured_error_extra_string(structured_error, "failure_attribution") {
        return crate::evidence_policy::FailureAttribution::parse(&raw)
            .map(|kind| kind.as_str().to_string())
            .or(Some(raw));
    }
    structured_error
        .and_then(|value| failure_attribution_for_structured_error_kind(&value.error_kind))
        .map(|kind| kind.as_str().to_string())
}

pub(crate) fn failure_attribution_for_error_text(
    error_text: &str,
) -> Option<crate::evidence_policy::FailureAttribution> {
    let trimmed = error_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(structured) = crate::skills::parse_structured_skill_error(trimmed) {
        if let Some(raw) = structured_error_extra_string(Some(&structured), "failure_attribution") {
            if let Some(kind) = crate::evidence_policy::FailureAttribution::parse(&raw) {
                return Some(kind);
            }
        }
        if let Some(kind) = failure_attribution_for_structured_error_kind(&structured.error_kind) {
            return Some(kind);
        }
    }

    let normalized = trimmed.to_ascii_lowercase().replace('-', "_");
    if normalized.contains("schema_validation_failed")
        || normalized.contains("schema validation")
        || normalized.contains("json schema")
        || normalized.contains("invalid schema")
    {
        return Some(crate::evidence_policy::FailureAttribution::SchemaError);
    }
    if normalized.contains("answer_verifier_required_evidence_block") {
        return Some(crate::evidence_policy::FailureAttribution::ContractGap);
    }
    if normalized.contains("all llm providers in circuit_breaker cooldown")
        || normalized.contains("unknown llm error")
        || (normalized.contains("provider=") && normalized.contains(" failed"))
        || normalized.contains("provider_error")
        || normalized.contains("provider_retryable")
        || normalized.contains("provider_non_retryable")
        || normalized.contains("rate_limited")
        || normalized.contains("quota_exhausted")
    {
        return Some(crate::evidence_policy::FailureAttribution::ProviderError);
    }
    if normalized.contains("channel_send_failed")
        || normalized.contains("delivery_error")
        || normalized.contains("delivery failed")
        || normalized.contains("send status=")
    {
        return Some(crate::evidence_policy::FailureAttribution::DeliveryError);
    }
    None
}

pub(super) fn failure_attribution_for_structured_error_kind(
    error_kind: &str,
) -> Option<crate::evidence_policy::FailureAttribution> {
    let normalized = error_kind.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "schema_error"
        | "schema_validation_failed"
        | "schema_recovery_failed"
        | "json_schema_error"
        | "invalid_json_schema"
        | "missing_required_field" => Some(crate::evidence_policy::FailureAttribution::SchemaError),
        "provider_error"
        | "provider_retryable_response"
        | "provider_retryable_business"
        | "provider_non_retryable_business"
        | "provider_response_invalid"
        | "provider_schema_error"
        | "provider_unavailable"
        | "transport_retryable"
        | "rate_limited"
        | "quota_exhausted"
        | "llm_provider_error"
        | "llm_provider_unavailable" => {
            Some(crate::evidence_policy::FailureAttribution::ProviderError)
        }
        "delivery_error"
        | "delivery_failed"
        | "channel_send_failed"
        | "file_delivery_failed"
        | "media_delivery_failed"
        | "missing_delivery_artifact"
        | "delivery_token_invalid" => {
            Some(crate::evidence_policy::FailureAttribution::DeliveryError)
        }
        "permission_denied" | "policy_denied" | "skill_disabled" | "requires_confirmation" => {
            Some(crate::evidence_policy::FailureAttribution::PermissionDenied)
        }
        "contract_action_rejected" | "contract_policy_violation" | "contract_missing" => {
            Some(crate::evidence_policy::FailureAttribution::ContractGap)
        }
        "budget_exhausted" | "round_budget_exhausted" | "tool_budget_exhausted" => {
            Some(crate::evidence_policy::FailureAttribution::BudgetExhausted)
        }
        "prompt_budget_error" => {
            Some(crate::evidence_policy::FailureAttribution::PromptBudgetError)
        }
        _ => None,
    }
}

pub(super) fn contract_policy_trace_json(
    structured_error: Option<&crate::skills::StructuredSkillError>,
) -> Option<Value> {
    let structured_error = structured_error?;
    if structured_error.error_kind != "contract_action_rejected" {
        return None;
    }
    let extra = structured_error.extra.as_ref()?;
    Some(json!({
        "decision": extra.get("decision").and_then(Value::as_str),
        "action": extra.get("action").and_then(Value::as_str),
        "contract_match": extra.get("contract_match").and_then(Value::as_str),
        "required_evidence": extra.get("required_evidence").cloned(),
        "preferred_actions": extra.get("preferred_actions").cloned(),
        "evidence_expression": extra.get("evidence_expression").cloned(),
        "final_answer_shape": extra.get("final_answer_shape").and_then(Value::as_str),
        "policy_mode": extra.get("policy_mode").and_then(Value::as_str),
        "evidence_scope": extra.get("evidence_scope").and_then(Value::as_str),
        "freshness": extra.get("freshness").and_then(Value::as_str),
        "artifact_kind": extra.get("artifact_kind").and_then(Value::as_str),
        "channel_visibility": extra.get("channel_visibility").and_then(Value::as_str),
        "evidence_profile": extra.get("evidence_profile").and_then(Value::as_str),
    }))
}
