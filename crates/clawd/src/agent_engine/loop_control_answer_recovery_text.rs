use super::*;

pub(super) fn deterministic_log_analyze_summary_text(
    _user_text: &str,
    findings: &[LogAnalyzeFinding],
) -> String {
    let mut sorted = findings.to_vec();
    sorted.sort_by(|left, right| {
        right
            .total_hits
            .cmp(&left.total_hits)
            .then_with(|| left.path.cmp(&right.path))
    });
    let top = &sorted[0];
    let mut lines = vec![
        "message_key=clawd.msg.log_analyze.summary".to_string(),
        "reason_code=log_analyze_observed_summary".to_string(),
        format!("analyzed_log_count={}", sorted.len()),
        format!("top_total_hits={}", top.total_hits),
    ];
    push_machine_line(&mut lines, "top_path", display_log_path(&top.path));
    push_machine_line(
        &mut lines,
        "top_keyword_counts",
        &format_keyword_counts(&top.keyword_counts),
    );
    for (idx, finding) in sorted.iter().take(4).enumerate() {
        let prefix = format!("finding.{}", idx + 1);
        push_machine_line(
            &mut lines,
            &format!("{prefix}.path"),
            display_log_path(&finding.path),
        );
        lines.push(format!("{prefix}.total_hits={}", finding.total_hits));
        push_machine_line(
            &mut lines,
            &format!("{prefix}.keyword_counts"),
            &format_keyword_counts(&finding.keyword_counts),
        );
    }
    lines.join("\n")
}

pub(super) fn deterministic_structured_search_summary_text(
    _user_text: &str,
    finding: &StructuredSearchFinding,
) -> String {
    let count = finding.count.max(finding.results.len());
    let mut lines = Vec::new();
    lines.push("message_key=clawd.msg.structured_search.candidates".to_string());
    lines.push("reason_code=structured_search_candidates".to_string());
    push_machine_line(&mut lines, "action", &finding.action);
    lines.push(format!("count={count}"));
    lines.push(format!("result_count={}", finding.results.len()));
    for (idx, result) in finding.results.iter().enumerate() {
        push_machine_line(&mut lines, &format!("candidate.{}", idx + 1), result);
    }
    lines.join("\n")
}

pub(super) fn deterministic_structured_count_summary_text(
    _user_text: &str,
    finding: &StructuredCountFinding,
) -> String {
    let mut lines = vec![
        "message_key=clawd.msg.structured_count.summary".to_string(),
        "reason_code=structured_count_observed".to_string(),
        format!("total={}", finding.total),
    ];
    if let Some(path) = finding.path.as_deref() {
        push_machine_line(&mut lines, "path", path);
    }
    if let Some(files) = finding.files {
        lines.push(format!("files={files}"));
    }
    if let Some(dirs) = finding.dirs {
        lines.push(format!("dirs={dirs}"));
    }
    if let Some(hidden) = finding.hidden {
        lines.push(format!("hidden={hidden}"));
    }
    if let Some(recursive) = finding.recursive {
        lines.push(format!("recursive={recursive}"));
        lines.push(format!("direct={}", !recursive));
    }
    lines.join("\n")
}

fn push_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let value = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    );
    lines.push(format!("{key}={value}"));
}

pub(super) fn display_log_path(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
}

pub(super) fn format_keyword_counts(counts: &[(String, u64)]) -> String {
    counts
        .iter()
        .take(5)
        .map(|(key, count)| format!("{key} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn mark_reply_failed_after_answer_verifier_exhausted(
    _user_text: &str,
    reply: &mut AskReply,
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) {
    let message = verifier.required_evidence_failure_payload_text();
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(message.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.record_final_answer(&message);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
        journal.record_final_failure_attribution_from_error(&message);
    }
    reply.text = message.clone();
    reply.messages = messages;
    reply.should_fail_task = true;
    reply.error_text = Some(message);
}

pub(super) fn try_accept_language_only_output_format_answer_verifier_gap(
    route_result: Option<&crate::answer_verifier::AnswerContract>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route.output_contract_is_unclassified()
        || route.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
    {
        return false;
    }
    if reply.text.trim().is_empty() {
        return false;
    }
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    let Some(summary) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if summary.pass
        || summary.missing_evidence_fields.is_empty()
        || !summary
            .missing_evidence_fields
            .iter()
            .all(|field| field == "output_format")
    {
        return false;
    }
    let recovered_answer = latest_publishable_terminal_answer(journal)
        .unwrap_or_else(|| reply.text.trim().to_string());
    if !recovered_answer.trim().is_empty() && reply.text.trim() != recovered_answer.trim() {
        reply.text = recovered_answer.clone();
        reply.messages = vec![recovered_answer.clone()];
    }
    journal.answer_verifier_summary = None;
    journal.record_final_answer(&recovered_answer);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_stop_signal(
        crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
    );
    reply.should_fail_task = false;
    reply.error_text = None;
    info!("answer_verifier_retry_exhausted_accepted_language_only_output_format_gap");
    true
}

fn latest_publishable_terminal_answer(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    journal.step_results.iter().rev().find_map(|step| {
        if !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
            || step.status != crate::executor::StepExecutionStatus::Ok
        {
            return None;
        }
        let answer = step.output_excerpt.as_deref()?.trim();
        if answer.is_empty()
            || serde_json::from_str::<serde_json::Value>(answer).is_ok()
            || crate::finalize::looks_like_planner_artifact(answer)
            || crate::finalize::looks_like_internal_trace_artifact(answer)
            || crate::finalize::is_execution_summary_message(answer)
        {
            return None;
        }
        Some(answer.to_string())
    })
}
