use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FilesystemMutationSuccessFinding {
    status: Option<String>,
    effective_status: Option<String>,
    result_kind: Option<String>,
    action: Option<String>,
    path: Option<String>,
    namespace: Option<String>,
    total_chunks: Option<u64>,
}

pub(super) fn try_recover_filesystem_mutation_success_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FilesystemMutationResult)
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() {
        return false;
    }
    if !verifier.missing_evidence_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "output_format" | "field_value" | "status" | "effective_status" | "unsupported_claims"
        )
    }) {
        return false;
    }
    let Some(finding) = observed_filesystem_mutation_success_findings(reply)
        .into_iter()
        .max_by(|left, right| {
            filesystem_mutation_success_score(left).cmp(&filesystem_mutation_success_score(right))
        })
    else {
        return false;
    };
    let answer = deterministic_filesystem_mutation_success_status_text(&finding);
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer.clone();
    reply.messages = vec![answer];
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_filesystem_mutation_success");
    true
}

fn observed_filesystem_mutation_success_findings(
    reply: &AskReply,
) -> Vec<FilesystemMutationSuccessFinding> {
    let mut findings = Vec::new();
    if let Some(value) = parse_machine_json_value(reply.text.trim()) {
        collect_filesystem_mutation_success_findings_from_value(&value, &mut findings);
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(value) = parse_machine_json_value(output.trim()) else {
            continue;
        };
        collect_filesystem_mutation_success_findings_from_value(&value, &mut findings);
    }
    findings.retain(filesystem_mutation_success_finding_is_success);
    findings.sort_by(|left, right| {
        filesystem_mutation_success_score(right)
            .cmp(&filesystem_mutation_success_score(left))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.namespace.cmp(&right.namespace))
    });
    findings.dedup();
    findings
}

fn collect_filesystem_mutation_success_findings_from_value(
    value: &Value,
    findings: &mut Vec<FilesystemMutationSuccessFinding>,
) {
    match value {
        Value::Object(object) => {
            if let Some(finding) = filesystem_mutation_success_finding_from_object(object) {
                findings.push(finding);
            }
            for key in ["extra", "result", "data", "payload"] {
                if let Some(child) = object.get(key) {
                    collect_filesystem_mutation_success_findings_from_value(child, findings);
                }
            }
            for key in ["steps", "results", "items"] {
                if let Some(items) = object.get(key).and_then(Value::as_array) {
                    for item in items {
                        collect_filesystem_mutation_success_findings_from_value(item, findings);
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_filesystem_mutation_success_findings_from_value(item, findings);
            }
        }
        _ => {}
    }
}

fn filesystem_mutation_success_finding_from_object(
    object: &serde_json::Map<String, Value>,
) -> Option<FilesystemMutationSuccessFinding> {
    let semantic_kind = object
        .get("semantic_kind")
        .and_then(Value::as_str)
        .map(str::trim);
    let action = first_string_field_or_array_item(object, &["action", "observed_actions"]);
    let result_kind = first_string_field_or_array_item(object, &["result_kind", "result_kinds"]);
    let effective_status = string_object_field(object, "effective_status");
    let status = string_object_field(object, "status");
    let effective_success = object
        .get("effective_success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let idempotent_success = object
        .get("idempotent_success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_filesystem_mutation_signal =
        matches!(semantic_kind, Some("filesystem_mutation_result"))
            || action
                .as_deref()
                .is_some_and(filesystem_mutation_success_action)
            || result_kind
                .as_deref()
                .is_some_and(filesystem_mutation_success_result_kind)
            || effective_success
            || idempotent_success;
    if !has_filesystem_mutation_signal {
        return None;
    }

    let finding = FilesystemMutationSuccessFinding {
        status,
        effective_status,
        result_kind,
        action,
        path: first_string_field_or_array_item(object, &["path", "paths", "resolved_path"]),
        namespace: first_string_field_or_array_item(object, &["namespace", "namespaces"]),
        total_chunks: object
            .get("stats")
            .and_then(|stats| stats.get("total_chunks"))
            .and_then(Value::as_u64),
    };
    filesystem_mutation_success_finding_is_success(&finding).then_some(finding)
}

fn filesystem_mutation_success_finding_is_success(
    finding: &FilesystemMutationSuccessFinding,
) -> bool {
    finding
        .effective_status
        .as_deref()
        .is_some_and(|status| status == "ok")
        || finding
            .result_kind
            .as_deref()
            .is_some_and(filesystem_mutation_success_result_kind)
        || (finding.status.as_deref() == Some("ok")
            && finding
                .action
                .as_deref()
                .is_some_and(filesystem_mutation_success_action)
            && !finding
                .result_kind
                .as_deref()
                .is_some_and(filesystem_mutation_non_success_result_kind))
}

fn filesystem_mutation_success_score(finding: &FilesystemMutationSuccessFinding) -> usize {
    [
        finding.status.as_deref(),
        finding.effective_status.as_deref(),
        finding.result_kind.as_deref(),
        finding.action.as_deref(),
        finding.path.as_deref(),
        finding.namespace.as_deref(),
    ]
    .into_iter()
    .flatten()
    .count()
        + usize::from(finding.total_chunks.is_some())
}

fn filesystem_mutation_success_action(action: &str) -> bool {
    matches!(
        action.trim(),
        "append_text"
            | "copy_path"
            | "create_dir"
            | "ingest"
            | "move_path"
            | "remove_path"
            | "replace_text"
            | "write_text"
            | "write_file"
    )
}

fn filesystem_mutation_success_result_kind(result_kind: &str) -> bool {
    matches!(
        result_kind.trim(),
        "already_indexed" | "updated" | "created" | "written" | "deleted" | "moved" | "copied"
    )
}

fn filesystem_mutation_non_success_result_kind(result_kind: &str) -> bool {
    matches!(
        result_kind.trim(),
        "no_new_documents" | "no_documents_indexed" | "needs_attention" | "failed" | "error"
    )
}

fn deterministic_filesystem_mutation_success_status_text(
    finding: &FilesystemMutationSuccessFinding,
) -> String {
    let mut parts = vec!["status=ok".to_string()];
    push_machine_status_part(
        &mut parts,
        "effective_status",
        finding.effective_status.as_deref(),
    );
    push_machine_status_part(&mut parts, "result_kind", finding.result_kind.as_deref());
    push_machine_status_part(&mut parts, "action", finding.action.as_deref());
    push_machine_status_part(&mut parts, "path", finding.path.as_deref());
    push_machine_status_part(&mut parts, "namespace", finding.namespace.as_deref());
    if let Some(total_chunks) = finding.total_chunks {
        parts.push(format!("total_chunks={total_chunks}"));
    }
    parts.join(" ")
}

fn push_machine_status_part(parts: &mut Vec<String>, key: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    parts.push(format!("{key}={}", machine_status_value(value)));
}

fn machine_status_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '/' | '@'))
    {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
    }
}

fn first_string_field_or_array_item(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        if let Some(text) = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_string());
        }
        value
            .as_array()
            .and_then(|items| {
                items.iter().find_map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                })
            })
            .map(ToString::to_string)
    })
}

fn string_object_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn parse_machine_json_value(output: &str) -> Option<Value> {
    let trimmed = output.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    serde_json::from_str::<Value>(trimmed).ok()
}
