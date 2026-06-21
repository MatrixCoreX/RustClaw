use serde_json::Value;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, OutputResponseShape};

pub(super) fn synthesize_answer_allows_direct_fallback(evidence_refs: &[String]) -> bool {
    evidence_refs.is_empty()
        || evidence_refs
            .iter()
            .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
}

pub(super) fn synthesize_route_allows_direct_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.route_result.as_ref()) else {
        return true;
    };
    if crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough(route) {
        return false;
    }
    if route.ask_mode.is_plain_act()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
    {
        return true;
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::ConfigValidation
    ) {
        return true;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
    {
        return false;
    }
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
            | crate::OutputResponseShape::Strict
            | crate::OutputResponseShape::FileToken
    ) || route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
}

pub(super) fn synthesize_route_prefers_model_language_observed_status(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|context| context.route_result.as_ref())
        .is_some_and(|route| {
            matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::CommandOutputSummary
                    | crate::OutputSemanticKind::ExecutionFailedStep
            ) && route.output_contract.requires_content_evidence
                && !route.output_contract.delivery_required
        })
}

fn output_has_count_inventory_total(output: &str) -> bool {
    let output = crate::agent_engine::observed_output::normalized_success_body_for_direct_answer(
        output.trim(),
    );
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    value.get("action").and_then(|value| value.as_str()) == Some("count_inventory")
        && value
            .get("counts")
            .and_then(|counts| counts.get("total"))
            .and_then(|value| value.as_u64())
            .is_some()
}

fn quantity_comparison_has_multiple_count_observations(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter(|output| output_has_count_inventory_total(output))
        .take(2)
        .count()
        >= 2
}

fn synthesize_direct_fallback_blocked_by_multi_count_quantity_comparison(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|context| context.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
                && quantity_comparison_has_multiple_count_observations(loop_state)
        })
}

pub(super) fn synthesize_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if let Some(answer) =
        synthesize_strict_raw_tail_read_direct_answer(loop_state, agent_run_context)
    {
        return Some(answer);
    }
    if agent_run_context
        .and_then(|context| context.route_result.as_ref())
        .is_some_and(
            crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough,
        )
    {
        return None;
    }
    if route_needs_synthesis_for_multi_observation_grounded_summary(loop_state, agent_run_context) {
        return None;
    }
    if synthesize_direct_fallback_blocked_by_multi_count_quantity_comparison(
        loop_state,
        agent_run_context,
    ) {
        return None;
    }
    crate::agent_engine::observed_output::extract_direct_answer_from_generic_output_i18n(
        loop_state,
        state,
        agent_run_context,
    )
    .or_else(|| {
        crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
            loop_state,
            state,
            agent_run_context,
        )
    })
    .map(|answer| answer.trim().to_string())
    .filter(|answer| !answer.is_empty())
}

pub(super) fn archive_database_aggregate_structured_answer(
    loop_state: &LoopState,
) -> Option<String> {
    let archive_list = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "archive_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("list"))?;
    let archive_read = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "archive_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("read"))?;
    let db_tables = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "db_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("list_tables"))?;

    let entries = archive_entry_names(&archive_list)?;
    let member = archive_read
        .get("member")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let content = archive_read
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let tables = sqlite_table_names(&db_tables)?;

    Some(
        serde_json::json!({
            "archive": {
                "entries": entries,
                "member": {
                    "name": member,
                    "content": content,
                },
            },
            "database": {
                "tables": tables,
            },
        })
        .to_string(),
    )
}

fn skill_output_payload(output: &str) -> Option<Value> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        return Some(extra.clone());
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        if let Ok(inner) = serde_json::from_str::<Value>(text.trim()) {
            return Some(inner);
        }
    }
    Some(value)
}

fn archive_entry_names(payload: &Value) -> Option<Vec<String>> {
    let mut names = Vec::new();
    if let Some(entries) = payload.get("entries").and_then(Value::as_array) {
        for entry in entries {
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                push_unique_string(&mut names, name);
            }
        }
    }
    if names.is_empty() {
        if let Some(candidates) = payload.get("candidates").and_then(Value::as_array) {
            for candidate in candidates {
                if let Some(name) = candidate.as_str() {
                    push_unique_string(&mut names, name);
                }
            }
        }
    }
    (!names.is_empty()).then_some(names)
}

fn sqlite_table_names(payload: &Value) -> Option<Vec<String>> {
    let rows = payload
        .get("result")
        .and_then(|result| result.get("rows"))
        .or_else(|| payload.get("rows"))
        .and_then(Value::as_array)?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(name) = row.get("name").and_then(Value::as_str) {
            push_unique_string(&mut names, name);
        }
    }
    (!names.is_empty()).then_some(names)
}

fn push_unique_string(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

fn synthesize_strict_raw_tail_read_direct_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route.output_contract.response_shape != crate::OutputResponseShape::Strict
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(strict_raw_tail_read_answer_from_output)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

fn strict_raw_tail_read_answer_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    strict_raw_tail_read_answer_from_value(&value)
}

fn strict_raw_tail_read_answer_from_value(value: &Value) -> Option<String> {
    if let Some(answer) = strict_raw_tail_read_answer_from_flat_value(value) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(strict_raw_tail_read_answer_from_value)
        .or_else(|| {
            value
                .get("text")
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .and_then(|inner| strict_raw_tail_read_answer_from_value(&inner))
        })
}

fn strict_raw_tail_read_answer_from_flat_value(value: &Value) -> Option<String> {
    if !matches!(
        value.get("action").and_then(Value::as_str),
        Some("read_range" | "read_text_range")
    ) || value.get("mode").and_then(Value::as_str) != Some("tail")
    {
        return None;
    }
    let requested_n = value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(Value::as_u64)?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(Value::as_str)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert("requested_n".to_string(), Value::Number(requested_n.into()));
    }
    crate::agent_engine::observed_output::tail_read_range_direct_answer_candidate(
        &candidate.to_string(),
        false,
    )
}

pub(super) fn synthesize_contract_matrix_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.route_result.as_ref())?;
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)?;
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ConfigMutation {
        return None;
    }
    if crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough(route) {
        return None;
    }
    if synthesize_direct_fallback_blocked_by_multi_count_quantity_comparison(
        loop_state,
        agent_run_context,
    ) {
        return None;
    }
    if synthesize_direct_fallback_would_passthrough_multiline_read_range(
        loop_state,
        agent_run_context,
    ) {
        return None;
    }
    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
}

fn route_needs_synthesis_for_multi_observation_grounded_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return false;
    }
    let Some(shape) =
        crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
    else {
        return false;
    };
    if shape.class() != crate::contract_matrix::FinalAnswerShapeClass::GroundedSummary {
        return false;
    }
    successful_observation_step_count(loop_state) >= 2
}

fn successful_observation_step_count(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| !output.is_empty())
        })
        .count()
}

pub(super) fn synthesize_direct_fallback_would_passthrough_multiline_read_range(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::Scalar
                | OutputResponseShape::Strict
                | OutputResponseShape::OneSentence
        )
    {
        return false;
    }
    let semantic_blocks_direct_passthrough = matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
            | crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
    ) || (route.output_contract.semantic_kind
        == crate::OutputSemanticKind::RawCommandOutput
        && latest_round_plan_requests_synthesis(loop_state));
    if !semantic_blocks_direct_passthrough {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find_map(multiline_read_range_content_line_count)
        .is_some_and(|line_count| line_count > 1)
}

fn latest_round_plan_requests_synthesis(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        round.plan_result.as_ref().is_some_and(|plan| {
            plan.steps
                .iter()
                .any(|step| step.action_type == "synthesize_answer")
        })
    })
}

fn multiline_read_range_content_line_count(output: &str) -> Option<usize> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let action = value.get("action").and_then(Value::as_str)?;
    if !matches!(action, "read_range" | "read_text_range") {
        return None;
    }
    let text = value
        .get("content")
        .or_else(|| value.get("excerpt"))
        .and_then(Value::as_str)?;
    Some(
        text.lines()
            .map(strip_markdown_read_line_prefix)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .count(),
    )
}

pub(super) fn deterministic_scalar_markdown_heading_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.output_contract.response_shape != OutputResponseShape::Scalar
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
        )
    {
        return None;
    }
    let output = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| {
            output.contains("\"read_range\"") || output.contains("\"read_text_range\"")
        })?;
    markdown_heading_from_read_output(output)
}

fn markdown_heading_from_read_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let text = value
        .get("content")
        .or_else(|| value.get("excerpt"))
        .and_then(Value::as_str)?;
    standalone_markdown_heading_from_text(text)
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

fn strip_markdown_read_line_prefix(line: &str) -> &str {
    let trimmed = line.trim();
    if let Some((prefix, rest)) = trimmed.split_once('|') {
        if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.trim();
        }
    }
    line
}

fn markdown_heading_from_line(line: &str) -> Option<String> {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed.get(hashes..)?.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn markdown_line_is_non_answer_separator_heading(line: &str) -> bool {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    trimmed.get(hashes..).map(str::trim).is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|ch| matches!(ch, '=' | '-' | '_' | '*' | '#'))
    })
}

pub(super) fn synthesize_user_language_source<'a>(
    user_text: &'a str,
    agent_run_context: Option<&'a AgentRunContext>,
) -> &'a str {
    agent_run_context
        .and_then(|context| {
            context
                .original_user_request
                .as_deref()
                .or(context.user_request.as_deref())
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(user_text)
}

pub(super) fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim())
        .filter(|intent| !intent.is_empty())
        .unwrap_or_default()
        .to_string()
}

pub(super) fn synthesize_failure_observed_facts(
    loop_state: &LoopState,
    refs_summary: &str,
) -> Vec<String> {
    let mut facts = vec![
        format!("synthesize_refs: {}", refs_summary.trim()),
        format!(
            "observed_steps_count: {}",
            loop_state.executed_step_results.len()
        ),
    ];
    let mut recent_steps = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
        })
        .take(4)
        .collect::<Vec<_>>();
    recent_steps.reverse();
    for step in recent_steps {
        let mut parts = vec![
            format!("skill={}", step.skill.trim()),
            format!("status={}", step.status.as_str()),
        ];
        if let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!(
                "output_excerpt={}",
                crate::truncate_for_agent_trace(output)
            ));
        }
        if let Some(error) = step
            .error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!(
                "error_summary={}",
                crate::truncate_for_agent_trace(error)
            ));
        }
        facts.push(format!("observed_step: {}", parts.join(", ")));
    }
    facts
}

pub(super) fn step_has_observable_synthesis_fact(
    step: &crate::executor::StepExecutionResult,
) -> bool {
    if step.is_ok() {
        return step
            .output
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
    }
    step.error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|err| {
            crate::skills::is_observable_run_cmd_error(&step.skill, err)
                || crate::skills::is_recoverable_skill_error(&step.skill, err)
        })
}

pub(super) fn synthesize_failure_should_replan(loop_state: &LoopState) -> bool {
    let previous_synthesis_failures = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.skill == "synthesize_answer" && !step.is_ok())
        .count();
    if previous_synthesis_failures > 0 {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "think" | "synthesize_answer"
        ) && step_has_observable_synthesis_fact(step)
    })
}
