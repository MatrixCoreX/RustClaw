use super::*;

pub(super) fn deterministic_log_analyze_summary_text(
    user_text: &str,
    findings: &[LogAnalyzeFinding],
) -> String {
    let prefer_english = crate::language_policy::request_language_hint(user_text) == "en";
    let mut sorted = findings.to_vec();
    sorted.sort_by(|left, right| {
        right
            .total_hits
            .cmp(&left.total_hits)
            .then_with(|| left.path.cmp(&right.path))
    });
    let top = &sorted[0];
    let overview = sorted
        .iter()
        .take(4)
        .map(|finding| {
            format!(
                "{}: {}",
                display_log_path(&finding.path),
                format_keyword_counts(&finding.keyword_counts)
            )
        })
        .collect::<Vec<_>>()
        .join(if prefer_english { "; " } else { "；" });
    if prefer_english {
        format!(
            "Most notable: `{}` has the heaviest recent signal ({}). Also checked other log files in the directory; summary: {}.",
            display_log_path(&top.path),
            format_keyword_counts(&top.keyword_counts),
            overview
        )
    } else {
        format!(
            "最值得注意的是 `{}`：{}，这是当前已分析日志里异常信号最重的文件；同时也看了 logs 目录里的其他日志，简要汇总：{}。",
            display_log_path(&top.path),
            format_keyword_counts(&top.keyword_counts),
            overview
        )
    }
}

pub(super) fn deterministic_structured_search_summary_text(
    user_text: &str,
    finding: &StructuredSearchFinding,
) -> String {
    let count = finding.count.max(finding.results.len());
    let prefer_english = crate::language_policy::request_language_hint(user_text) == "en";
    let mut lines = Vec::new();
    if prefer_english {
        lines.push(format!("Found {count} candidates:"));
    } else {
        lines.push(format!("找到 {count} 个候选："));
    }
    for (idx, result) in finding.results.iter().enumerate() {
        lines.push(format!("{}. {}", idx + 1, result));
    }
    lines.join("\n")
}

pub(super) fn deterministic_rss_news_items_text(items: &[RssNewsItem]) -> String {
    items
        .iter()
        .take(12)
        .enumerate()
        .map(|(idx, item)| {
            let mut fields = vec![
                format!("title={}", item.title),
                format!("source_host={}", item.source_host),
            ];
            if let Some(date) = item.date.as_deref() {
                fields.push(format!("date={date}"));
            }
            format!("{}. {}", idx + 1, fields.join(" | "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn deterministic_structured_count_summary_text(
    user_text: &str,
    finding: &StructuredCountFinding,
) -> String {
    let prefer_english = crate::language_policy::request_language_hint(user_text) == "en";
    let scope = finding.path.as_deref().unwrap_or(if prefer_english {
        "the requested scope"
    } else {
        "目标范围"
    });
    let direct = finding.recursive == Some(false);
    match (
        prefer_english,
        direct,
        finding.files,
        finding.dirs,
        finding.hidden,
    ) {
        (true, true, Some(files), Some(dirs), Some(hidden)) => format!(
            "{scope} has {} direct entries: {files} files, {dirs} directories, {hidden} hidden.",
            finding.total
        ),
        (true, true, Some(files), Some(dirs), None) => format!(
            "{scope} has {} direct entries: {files} files and {dirs} directories.",
            finding.total
        ),
        (true, _, Some(files), Some(dirs), _) => format!(
            "{scope} has {} entries: {files} files and {dirs} directories.",
            finding.total
        ),
        (true, true, _, _, _) => {
            format!("{scope} has {} direct entries.", finding.total)
        }
        (true, _, _, _, _) => format!("{scope} has {} entries.", finding.total),
        (false, true, Some(files), Some(dirs), Some(hidden)) => format!(
            "{scope} 共有 {} 个直接子项：文件 {files} 个，目录 {dirs} 个，隐藏项 {hidden} 个。",
            finding.total
        ),
        (false, true, Some(files), Some(dirs), None) => format!(
            "{scope} 共有 {} 个直接子项：文件 {files} 个，目录 {dirs} 个。",
            finding.total
        ),
        (false, _, Some(files), Some(dirs), _) => format!(
            "{scope} 共有 {} 个子项：文件 {files} 个，目录 {dirs} 个。",
            finding.total
        ),
        (false, true, _, _, _) => format!("{scope} 共有 {} 个直接子项。", finding.total),
        (false, _, _, _, _) => format!("{scope} 共有 {} 个子项。", finding.total),
    }
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
    route_result: Option<&RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
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
    journal.answer_verifier_summary = None;
    journal.record_final_answer(&reply.text);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    reply.should_fail_task = false;
    reply.error_text = None;
    info!("answer_verifier_retry_exhausted_accepted_language_only_output_format_gap");
    true
}
