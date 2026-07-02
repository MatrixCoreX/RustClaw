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

pub(crate) fn evidence_coverage_for_route(
    route: &crate::RouteResult,
    journal: &TaskJournal,
) -> TaskJournalEvidenceCoverage {
    let required_evidence = crate::evidence_policy::required_evidence_fields_for_route(route);
    let (observed_fields, mut observed_canonical, observed_extractors, observed_evidence_sources) =
        observed_evidence_field_sets(journal);
    augment_route_canonical_evidence(
        route,
        &observed_fields,
        &observed_extractors,
        &mut observed_canonical,
    );
    let effective_output_contract = route.effective_output_contract();
    let evidence_expression = crate::evidence_policy::bundled_contract_matrix()
        .and_then(|matrix| matrix.match_output_contract(&effective_output_contract))
        .map(|matched| matched.evidence_expression());
    let missing_evidence = evidence_expression
        .as_ref()
        .map(|expression| missing_evidence_for_expression(expression, &observed_canonical))
        .unwrap_or_else(|| missing_required_evidence(&required_evidence, &observed_canonical));
    let confidence = evidence_coverage_confidence(&required_evidence, &missing_evidence);
    let repair_eligible = evidence_coverage_repair_eligible(route, &missing_evidence);
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
    route: &crate::RouteResult,
    missing_evidence: &[String],
) -> bool {
    !missing_evidence.is_empty() && !route.output_contract.delivery_required
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

pub(super) fn evidence_coverage_trace_json(
    route: &crate::RouteResult,
    journal: &TaskJournal,
) -> Value {
    evidence_coverage_for_route(route, journal).to_trace_json()
}

pub(super) fn task_outcome_summary_json(journal: &TaskJournal) -> Value {
    let final_shape = journal
        .route_result
        .as_ref()
        .and_then(crate::evidence_policy::trace_snapshot_for_route)
        .and_then(|snapshot| {
            snapshot
                .get("final_answer_shape")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
    let missing_evidence = journal
        .route_result
        .as_ref()
        .map(|route| evidence_coverage_for_route(route, journal).missing_evidence)
        .unwrap_or_default();
    let missing_count = missing_evidence.len();
    let state = match journal.final_status {
        Some(TaskJournalFinalStatus::Success) if missing_count == 0 => "completed",
        Some(TaskJournalFinalStatus::Success) => "needs_attention",
        Some(TaskJournalFinalStatus::Clarify) => "needs_input",
        Some(TaskJournalFinalStatus::Failure | TaskJournalFinalStatus::ResumeFailure) => "failed",
        None => "in_progress",
    };
    let (message_zh, message_en, next_step_zh, next_step_en) = match state {
        "completed" => (
            "任务已完成。",
            "The task completed.",
            "可以直接查看结果。",
            "You can review the result.",
        ),
        "needs_attention" => (
            "任务已返回结果，但证据没有完全匹配。",
            "The task returned a result, but evidence did not fully match.",
            "请展开技术详情确认缺少的证据，必要时补充目标后重试。",
            "Open technical details to check missing evidence, then add the target and retry if needed.",
        ),
        "needs_input" => (
            "任务需要你补充信息。",
            "The task needs more information.",
            "请按提示补充目标、路径或确认信息。",
            "Provide the requested target, path, or confirmation.",
        ),
        "failed" if missing_count > 0 => (
            "任务没有完成，缺少必要证据。",
            "The task did not complete because required evidence is missing.",
            "请补充明确目标后重试。",
            "Add a clearer target and retry.",
        ),
        "failed" => (
            "任务没有完成。",
            "The task did not complete.",
            "请根据错误信息处理后重试；技术详情已保留在下方。",
            "Use the error message to decide the next step, then retry. Technical details are available below.",
        ),
        _ => (
            "任务正在处理。",
            "The task is in progress.",
            "稍后重新查询任务状态。",
            "Query the task again shortly.",
        ),
    };
    json!({
        "schema_version": 1,
        "state": state,
        "message_zh": message_zh,
        "message_en": message_en,
        "next_step_zh": next_step_zh,
        "next_step_en": next_step_en,
        "final_answer_shape": final_shape,
        "missing_evidence_count": missing_count,
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
        if !step_can_supply_contract_evidence(step, journal.route_result.as_ref()) {
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

pub(super) fn augment_route_canonical_evidence(
    route: &crate::RouteResult,
    observed_fields: &BTreeSet<String>,
    observed_extractors: &BTreeSet<String>,
    observed_canonical: &mut BTreeSet<String>,
) {
    if route.output_contract_marker_is(crate::OutputSemanticKind::ConfigMutation) {
        if observed_field_present(observed_fields, "valid")
            || observed_field_present(observed_fields, "validated")
            || config_mutation_plan_fields_present(observed_fields)
            || observed_extractors.contains("config_edit.plan_config_change.structured_json_v1")
        {
            observed_canonical.insert("valid".to_string());
        }
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
        && observed_canonical.contains("size_bytes")
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::GitCommitSubject,
        crate::OutputSemanticKind::GitRepositoryState,
    ]) && (observed_canonical.contains("command_output")
        || observed_canonical.contains("content_excerpt")
        || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
        && (observed_canonical.contains("content_excerpt")
            || observed_canonical.contains("field_value")
            || observed_fields.contains("excerpt")
            || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("command_output".to_string());
    }
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::FileNames,
        crate::OutputSemanticKind::FilePaths,
    ]) && observed_canonical.contains("content_match")
        && observed_canonical.contains("path")
    {
        observed_canonical.insert("candidates".to_string());
    }
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::ScalarPathOnly,
        crate::OutputSemanticKind::FileBasename,
    ]) && (observed_canonical.contains("path")
        || observed_canonical.contains("content_match")
        || observed_canonical.contains("candidates")
        || observed_field_with_prefix(observed_fields, "results["))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::RecentArtifactsJudgment)
        && (observed_canonical.contains("content_excerpt")
            || observed_canonical.contains("content_match")
            || observed_fields.contains("excerpt")
            || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        && (observed_canonical.contains("value") || observed_canonical.contains("field_value"))
    {
        observed_canonical.insert("count".to_string());
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::StructuredKeys)
        && (observed_canonical.contains("keys")
            || observed_field_with_prefix(observed_fields, "keys["))
    {
        observed_canonical.insert("field_value".to_string());
    }
    let docker_route_shape =
        route_has_docker_answer_shape(route) || route_has_unshaped_docker_compat_marker(route);
    let observed_textual_runtime_output = observed_canonical.contains("command_output")
        || observed_canonical.contains("content_excerpt")
        || observed_fields.contains("text_excerpt");
    if docker_route_shape && observed_textual_runtime_output {
        if route_has_docker_field_value_answer_shape(route)
            || route_has_unshaped_docker_field_value_compat_marker(route)
        {
            observed_canonical.insert("field_value".to_string());
        } else if route_has_docker_candidate_answer_shape(route) {
            observed_canonical.insert("candidates".to_string());
        }
    }
    if (route.output_contract_marker_is(crate::OutputSemanticKind::ServiceStatus)
        || route_has_service_status_answer_shape(route)
        || route_has_unshaped_service_status_compat_marker(route))
        && (observed_canonical.contains("status")
            || observed_canonical.contains("command_output")
            || observed_canonical.contains("content_excerpt")
            || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if (route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
    ]) || route_has_browser_http_excerpt_capability_marker(route))
        && http_response_body_fields_present(observed_fields)
    {
        observed_canonical.insert("content_excerpt".to_string());
    }
    if crate::machine_capability_ref::route_has_capability_namespace(route, &["x"])
        && (observed_canonical.contains("command_output")
            || observed_canonical.contains("content_excerpt")
            || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::SqliteDatabaseKindJudgment)
        && (observed_canonical.contains("candidates")
            || observed_fields.contains("rows")
            || observed_fields.contains("columns"))
    {
        observed_canonical.insert("field_value".to_string());
    }
}

pub(super) fn observed_field_with_prefix(observed_fields: &BTreeSet<String>, prefix: &str) -> bool {
    observed_fields
        .iter()
        .any(|field| field.starts_with(prefix))
}

fn route_final_answer_shape(
    route: &crate::RouteResult,
) -> Option<crate::evidence_policy::FinalAnswerShape> {
    crate::evidence_policy::final_answer_shape_for_route(route)
}

fn route_has_docker_answer_shape(route: &crate::RouteResult) -> bool {
    route_has_docker_field_value_answer_shape(route)
        || route_has_docker_candidate_answer_shape(route)
}

fn route_has_docker_field_value_answer_shape(route: &crate::RouteResult) -> bool {
    matches!(
        route_final_answer_shape(route),
        Some(crate::evidence_policy::FinalAnswerShape::LifecycleResult)
    )
}

fn route_has_docker_candidate_answer_shape(route: &crate::RouteResult) -> bool {
    matches!(
        route_final_answer_shape(route),
        Some(
            crate::evidence_policy::FinalAnswerShape::ContainerList
                | crate::evidence_policy::FinalAnswerShape::ImageList
                | crate::evidence_policy::FinalAnswerShape::LogExcerptOrSummary
        )
    )
}

fn route_has_unshaped_docker_compat_marker(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action(
        route,
        &["docker"],
        &["inspect", "version"],
    )
}

fn route_has_unshaped_docker_field_value_compat_marker(route: &crate::RouteResult) -> bool {
    route_has_unshaped_docker_compat_marker(route)
}

fn route_has_service_status_answer_shape(route: &crate::RouteResult) -> bool {
    matches!(
        route_final_answer_shape(route),
        Some(
            crate::evidence_policy::FinalAnswerShape::LifecycleResult
                | crate::evidence_policy::FinalAnswerShape::StatusWithSource
        )
    )
}

fn route_has_unshaped_service_status_compat_marker(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action(
        route,
        &["service", "service_control"],
        &["logs", "verify"],
    ) || crate::machine_capability_ref::route_has_capability_action(
        route,
        &["system", "system_basic"],
        &["runtime_status"],
    )
}

fn route_has_browser_http_excerpt_capability_marker(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action(
        route,
        &["browser", "http", "web"],
        &["extract", "get", "open", "read"],
    )
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
    route: Option<&crate::RouteResult>,
) -> bool {
    if matches!(
        step.skill.as_str(),
        "respond" | "synthesize_answer" | "think" | "answer_verifier"
    ) {
        return false;
    }
    if route.is_some_and(|route| {
        !route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && !route.wants_file_delivery
    }) && step_reads_text_content(step)
    {
        return false;
    }
    match step.status {
        crate::executor::StepExecutionStatus::Ok => true,
        crate::executor::StepExecutionStatus::Error => {
            step_error_supplies_negative_contract_evidence(step, route)
                || step.skill == "run_cmd"
                    && route.is_some_and(|route| {
                        route.output_contract_marker_is(
                            crate::OutputSemanticKind::ExecutionFailedStep,
                        ) || crate::evidence_policy::required_evidence_fields_for_output_contract(
                            &route.effective_output_contract(),
                        )
                        .iter()
                        .any(|field| field == "command_output")
                    })
        }
    }
}

pub(super) fn step_error_supplies_negative_contract_evidence(
    step: &TaskJournalStepTrace,
    route: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    if !route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::ContentPresenceCheck,
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
        crate::OutputSemanticKind::ExcerptKindJudgment,
        crate::OutputSemanticKind::ExistenceWithPath,
        crate::OutputSemanticKind::ExistenceWithPathSummary,
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
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_evidence_field);
    match step.skill.as_str() {
        "fs_basic" | "system_basic" => matches!(
            action.as_deref(),
            Some("read_range" | "read_text_range" | "read_file" | "read")
        ),
        "archive_basic" => matches!(action.as_deref(), Some("read")),
        _ => false,
    }
}

pub(super) fn observed_field_present(observed_fields: &BTreeSet<String>, field: &str) -> bool {
    observed_fields.contains(field) || observed_fields.contains(&format!("extra.{field}"))
}

pub(super) fn config_mutation_plan_fields_present(observed_fields: &BTreeSet<String>) -> bool {
    observed_field_present(observed_fields, "path")
        && observed_field_present(observed_fields, "field_path")
        && observed_field_present(observed_fields, "new_value")
        && observed_field_present(observed_fields, "would_change")
        && observed_field_present(observed_fields, "requires_confirmation")
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
            ][..],
        ),
        (
            "content_match",
            &[
                "content_match",
                "match",
                "matches",
                "grep_matches",
                "lines",
                "results",
            ][..],
        ),
        (
            "path",
            &[
                "path",
                "resolved_path",
                "file_path",
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
        (
            "directory_structure",
            &[
                "child_count",
                "children",
                "directory_structure",
                "names_by_kind",
                "omitted_children",
                "tree",
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
        Some("results" | "paths" | "candidates" | "entries")
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
