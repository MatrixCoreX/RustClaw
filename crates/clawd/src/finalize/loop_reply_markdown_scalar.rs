use tracing::info;

use crate::agent_engine::{AgentRunContext, LoopState};

use super::{
    log_deterministic_delivery_record, message_is_non_answer_separator,
    route_allows_direct_scalar_observed_answer,
};

pub(super) fn deterministic_scalar_markdown_heading_answer_from_loop(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_direct_scalar_observed_answer(route)
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
        )
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| output.contains("\"read_range\"") || output.contains("\"read_text_range\""))
        .and_then(markdown_heading_from_read_output)
}

pub(super) fn route_allows_observed_markdown_heading_scalar_delivery(
    route: &crate::RouteResult,
) -> bool {
    if route_allows_direct_scalar_observed_answer(route) {
        return true;
    }
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
        && !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

fn route_allows_observed_markdown_heading_body_reduction(route: &crate::RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
        && !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

pub(super) fn observed_markdown_heading_scalar_answer_for_delivery(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery: &str,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_observed_markdown_heading_scalar_delivery(route)
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ExistenceWithPathSummary
        )
    {
        return None;
    }
    let trimmed_delivery = delivery.trim();
    if trimmed_delivery.is_empty() {
        return None;
    }
    let observed_output = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| {
            output.contains("\"read_range\"") || output.contains("\"read_text_range\"")
        })?;
    if trimmed_delivery.contains('\n') {
        if route.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && route_allows_observed_markdown_heading_body_reduction(route)
            && markdown_read_body_matches_delivery(observed_output, trimmed_delivery)
        {
            return first_markdown_heading_from_read_output(observed_output);
        }
        return None;
    }
    let observed_heading = markdown_heading_from_read_output(observed_output)?;
    if trimmed_delivery == observed_heading.trim() {
        return Some(observed_heading);
    }
    let delivery_heading = markdown_heading_from_line(trimmed_delivery)?;
    (delivery_heading.trim() == observed_heading.trim()).then_some(observed_heading)
}

pub(super) fn replace_delivery_with_observed_markdown_heading_scalar(
    task_id: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(current_delivery) = delivery_messages.last().map(String::as_str) else {
        return false;
    };
    let Some(answer) = observed_markdown_heading_scalar_answer_for_delivery(
        loop_state,
        agent_run_context,
        current_delivery,
    ) else {
        return false;
    };
    if current_delivery.trim() == answer.trim() {
        return false;
    }
    info!(
        "delivery markdown_heading_scalar_from_observed task_id={} previous={} observed={}",
        task_id,
        crate::truncate_for_log(current_delivery),
        crate::truncate_for_log(&answer)
    );
    log_deterministic_delivery_record(
        task_id,
        "markdown_heading_scalar_from_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    true
}

pub(super) fn markdown_heading_from_read_output(output: &str) -> Option<String> {
    let text = markdown_text_from_read_output(output)?;
    standalone_markdown_heading_from_text(&text)
}

pub(super) fn first_markdown_heading_from_read_output(output: &str) -> Option<String> {
    let text = markdown_text_from_read_output(output)?;
    text.lines().find_map(markdown_heading_from_line)
}

fn standalone_markdown_heading_from_text(text: &str) -> Option<String> {
    let mut heading: Option<String> = None;
    for line in text.lines() {
        let stripped = strip_markdown_read_line_prefix(line).trim();
        if stripped.is_empty() {
            continue;
        }
        if let Some(candidate) = markdown_heading_from_line(stripped) {
            if heading.is_some() {
                return None;
            }
            heading = Some(candidate);
            continue;
        }
        if markdown_line_is_non_answer_separator_heading(stripped) {
            continue;
        }
        return None;
    }
    heading
}

fn markdown_read_body_matches_delivery(output: &str, delivery: &str) -> bool {
    let Some(observed) = markdown_text_from_read_output(output) else {
        return false;
    };
    normalize_markdown_body_for_compare(&observed) == normalize_markdown_body_for_compare(delivery)
}

fn markdown_text_from_read_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    markdown_text_from_read_value(&value)
}

fn markdown_text_from_read_value(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value
        .get("content")
        .or_else(|| value.get("excerpt"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(
            text.lines()
                .map(strip_markdown_read_line_prefix)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if let Some(text) = value
        .get("extra")
        .and_then(markdown_text_from_read_value)
        .filter(|text| !text.trim().is_empty())
    {
        return Some(text);
    }
    None
}

fn normalize_markdown_body_for_compare(text: &str) -> String {
    text.lines()
        .map(strip_markdown_read_line_prefix)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .replace("\r\n", "\n")
}

fn strip_markdown_read_line_prefix(line: &str) -> &str {
    let trimmed = line.trim();
    if let Some((prefix, rest)) = trimmed.split_once('|') {
        if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.trim();
        }
    }
    line
}

pub(super) fn markdown_heading_from_line(line: &str) -> Option<String> {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed.get(hashes..)?.trim();
    if rest.is_empty() || message_is_non_answer_separator(rest) {
        return None;
    }
    Some(rest.to_string())
}

fn markdown_line_is_non_answer_separator_heading(line: &str) -> bool {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    trimmed
        .get(hashes..)
        .map(str::trim)
        .is_some_and(message_is_non_answer_separator)
}
