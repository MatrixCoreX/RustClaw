use super::*;

pub(crate) fn try_recover_structured_listing_answer_verifier_gap(
    route_result: Option<&crate::answer_verifier::AnswerContract>,
    reply: &mut AskReply,
) -> bool {
    if !route_allows_structured_listing_recovery(route_result, reply) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !structured_search_verifier_requests_full_candidates(verifier)
    {
        return false;
    }
    let Some(finding) = observed_structured_listing_findings(reply)
        .into_iter()
        .max_by(|left, right| {
            left.total_len()
                .cmp(&right.total_len())
                .then_with(|| left.dirs.len().cmp(&right.dirs.len()))
                .then_with(|| left.files.len().cmp(&right.files.len()))
        })
    else {
        return false;
    };
    if finding.total_len() == 0 {
        return false;
    }
    let answer = deterministic_structured_listing_grouped_text(&finding);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    tracing::info!(
        "answer_verifier_retry_recovered_with_structured_listing dirs={} files={} other={}",
        finding.dirs.len(),
        finding.files.len(),
        finding.other.len()
    );
    true
}

fn route_allows_structured_listing_recovery(
    route_result: Option<&crate::answer_verifier::AnswerContract>,
    reply: &AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route_requires_artifact_delivery(route) {
        return false;
    }
    if reply.task_journal.as_ref().is_some_and(|journal| {
        journal
            .step_results
            .iter()
            .any(crate::task_journal::step_reads_text_content)
    }) {
        return false;
    }
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::DirectoryEntryGroups,
        crate::OutputSemanticKind::DirectoryNames,
        crate::OutputSemanticKind::FileNames,
    ]) {
        return true;
    }
    if matches!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
    ) {
        return true;
    }
    observed_structured_listing_findings(reply)
        .into_iter()
        .any(|finding| finding.total_len() > 0)
}

fn route_requires_artifact_delivery(route: &crate::answer_verifier::AnswerContract) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle | crate::OutputDeliveryIntent::DirectoryBatchFiles
    ) || route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::GeneratedFileDelivery,
        crate::OutputSemanticKind::GeneratedFilePathReport,
    ])
}

#[derive(Debug, Clone, Default)]
struct StructuredListingFinding {
    dirs: Vec<String>,
    files: Vec<String>,
    other: Vec<String>,
}

impl StructuredListingFinding {
    fn total_len(&self) -> usize {
        self.dirs.len() + self.files.len() + self.other.len()
    }
}

fn observed_structured_listing_findings(reply: &AskReply) -> Vec<StructuredListingFinding> {
    let mut findings = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if !matches!(
            step.skill.as_str(),
            "fs_basic" | "system_basic" | "list_dir"
        ) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
            continue;
        };
        collect_structured_listing_findings_from_value(&value, &mut findings);
    }
    findings
}

fn collect_structured_listing_findings_from_value(
    value: &serde_json::Value,
    findings: &mut Vec<StructuredListingFinding>,
) {
    if let Some(finding) = parse_structured_listing_finding(value) {
        findings.push(finding);
    }
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if let Some(finding) = parse_structured_listing_finding(extra) {
            findings.push(finding);
        }
    }
}

fn parse_structured_listing_finding(value: &serde_json::Value) -> Option<StructuredListingFinding> {
    if !value_is_structured_listing(value) {
        return None;
    }
    let mut finding = StructuredListingFinding::default();
    collect_names_by_kind(value, &mut finding);
    collect_entry_names_by_kind(value, &mut finding);
    collect_plain_names(value, &mut finding);
    (finding.total_len() > 0).then_some(finding)
}

fn value_is_structured_listing(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(serde_json::Value::as_str),
        Some("inventory_dir" | "list_dir")
    ) || value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
        .is_some()
        || value
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .is_some()
        || value
            .get("names")
            .and_then(serde_json::Value::as_array)
            .is_some()
}

fn collect_names_by_kind(value: &serde_json::Value, finding: &mut StructuredListingFinding) {
    let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    push_json_string_array(names_by_kind.get("dirs"), &mut finding.dirs);
    push_json_string_array(names_by_kind.get("files"), &mut finding.files);
    push_json_string_array(names_by_kind.get("other"), &mut finding.other);
}

fn collect_entry_names_by_kind(value: &serde_json::Value, finding: &mut StructuredListingFinding) {
    let Some(entries) = value.get("entries").and_then(serde_json::Value::as_array) else {
        return;
    };
    for entry in entries {
        let Some(name) = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        match entry
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
        {
            Some("dir" | "directory" | "folder") => push_unique(&mut finding.dirs, name),
            Some("file") => push_unique(&mut finding.files, name),
            _ => push_unique(&mut finding.other, name),
        }
    }
}

fn collect_plain_names(value: &serde_json::Value, finding: &mut StructuredListingFinding) {
    if finding.total_len() > 0 {
        return;
    }
    let target = if value
        .get("dirs_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        &mut finding.dirs
    } else if value
        .get("files_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        &mut finding.files
    } else {
        &mut finding.other
    };
    push_json_string_array(value.get("names"), target);
}

fn push_json_string_array(value: Option<&serde_json::Value>, out: &mut Vec<String>) {
    let Some(items) = value.and_then(serde_json::Value::as_array) else {
        return;
    };
    for item in items {
        if let Some(name) = item.as_str().map(str::trim).filter(|name| !name.is_empty()) {
            push_unique(out, name);
        }
    }
}

fn push_unique(out: &mut Vec<String>, value: &str) {
    if !out.iter().any(|existing| existing == value) {
        out.push(value.to_string());
    }
}

fn deterministic_structured_listing_grouped_text(finding: &StructuredListingFinding) -> String {
    let mut lines = Vec::new();
    push_structured_listing_group("dirs", &finding.dirs, &mut lines);
    push_structured_listing_group("files", &finding.files, &mut lines);
    push_structured_listing_group("other", &finding.other, &mut lines);
    lines.join("\n")
}

fn push_structured_listing_group(label: &str, items: &[String], lines: &mut Vec<String>) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{label}.count={}", items.len()));
    lines.push(format!("{label}:"));
    lines.extend(items.iter().map(|item| format!("- {item}")));
}
