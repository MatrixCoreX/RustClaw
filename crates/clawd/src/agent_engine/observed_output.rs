use serde::Deserialize;

use super::{AgentRunContext, LoopState};
use crate::{llm_gateway, AppState, ClaimedTask};

const OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/observed_answer_fallback_prompt.md");
const OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH: &str =
    "prompts/observed_answer_fallback_prompt.md";

#[derive(Debug, Clone)]
pub(crate) struct GenericObservedOutput {
    pub(crate) skill: String,
    #[cfg(test)]
    pub(crate) action_label: String,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
struct ObservedAnswerFallbackOut {
    #[serde(default)]
    answer: String,
    #[serde(default)]
    qualified: bool,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    confidence: f64,
    #[serde(default, rename = "reason")]
    _reason: String,
}

fn latest_successful_step_index<F>(loop_state: &LoopState, predicate: F) -> Option<usize>
where
    F: Fn(&crate::executor::StepExecutionResult) -> bool,
{
    loop_state
        .executed_step_results
        .iter()
        .rposition(|step| step.is_ok() && predicate(step))
}

#[cfg(test)]
fn latest_successful_step_output<F>(loop_state: &LoopState, predicate: F) -> Option<String>
where
    F: Fn(&crate::executor::StepExecutionResult) -> bool,
{
    latest_successful_step_index(loop_state, predicate).and_then(|idx| {
        loop_state.executed_step_results[idx]
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
    })
}

#[cfg(test)]
pub(crate) fn extract_latest_successful_read_file_output(loop_state: &LoopState) -> Option<String> {
    latest_successful_step_output(loop_state, |step| step.skill == "read_file")
}

#[cfg(test)]
pub(crate) fn extract_latest_successful_list_dir_output(loop_state: &LoopState) -> Option<String> {
    latest_successful_step_output(loop_state, |step| step.skill == "list_dir")
}

pub(crate) fn extract_latest_generic_successful_output(
    loop_state: &LoopState,
) -> Option<GenericObservedOutput> {
    let idx = latest_successful_step_index(loop_state, |step| {
        if matches!(step.skill.as_str(), "read_file" | "list_dir") {
            return false;
        }
        let body = step.output.as_deref().map(str::trim).unwrap_or_default();
        !body.is_empty()
            && (crate::finalizer::classify_observed_content_status(body)
                == crate::finalizer::ObservedContentStatus::ContentAvailable
                || structured_scalar_candidate(&step.skill, body).is_some())
    })?;
    let step = &loop_state.executed_step_results[idx];
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    Some(GenericObservedOutput {
        skill: step.skill.clone(),
        #[cfg(test)]
        action_label: format!("{} skill({}): success", step.step_id, step.skill),
        body: body.to_string(),
    })
}

fn trim_for_observed_prompt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str("\n...[truncated]");
    out
}

fn normalized_scalar_candidate(body: &str) -> Option<String> {
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .collect::<Vec<_>>();
    (lines.len() == 1).then(|| lines[0].to_string())
}

fn structured_scalar_candidate(skill: &str, body: &str) -> Option<String> {
    if skill != "system_basic" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "extract_field" => {
            let field_path = value
                .get("field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("requested field");
            if value
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let text = value
                    .get("value_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
                return value.get("value").and_then(|v| match v {
                    serde_json::Value::Null => Some("null".to_string()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => None,
                });
            }
            Some(format!("{field_path} 字段不存在"))
        }
        "count_inventory" => {
            value
                .get("counts")
                .and_then(|v| v.get("total"))
                .and_then(|v| match v {
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
        }
        _ => None,
    }
}

fn normalize_read_range_excerpt(excerpt: &str) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn compact_log_analyze_excerpt(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let keyword_counts = value
        .get("keyword_counts")
        .and_then(|v| v.as_object())
        .map(|map| {
            let mut pairs = map
                .iter()
                .filter_map(|(key, count)| count.as_u64().map(|count| (key.as_str(), count)))
                .collect::<Vec<_>>();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            pairs
                .into_iter()
                .map(|(key, count)| format!("{key}={count}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let recent_matches = value
        .get("recent_matches")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let total_lines = value
        .get("total_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();

    let mut sections = vec![format!("log_analyze path={path} total_lines={total_lines}")];
    if !keyword_counts.is_empty() {
        sections.push(format!("keyword_counts: {}", keyword_counts.join(", ")));
    }
    if !recent_matches.is_empty() {
        sections.push(format!("recent_matches:\n- {}", recent_matches.join("\n- ")));
    }
    Some(sections.join("\n"))
}

fn structured_observed_body(skill: &str, body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    match skill {
        "system_basic" => {
            let action = value.get("action").and_then(|v| v.as_str())?;
            match action {
                "read_range" => value
                    .get("excerpt")
                    .and_then(|v| v.as_str())
                    .and_then(normalize_read_range_excerpt),
                _ => None,
            }
        }
        "log_analyze" => compact_log_analyze_excerpt(&value),
        _ => None,
    }
}

pub(crate) fn extract_direct_scalar_from_generic_output(loop_state: &LoopState) -> Option<String> {
    let observed_output = extract_latest_generic_successful_output(loop_state)?;
    let answer = structured_scalar_candidate(&observed_output.skill, &observed_output.body)
        .or_else(|| normalized_scalar_candidate(&observed_output.body))?;
    if crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some(answer)
}

fn observed_step_body(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    if let Some(normalized) = structured_observed_body(&step.skill, body) {
        return Some(normalized);
    }
    (crate::finalizer::classify_observed_content_status(body)
        == crate::finalizer::ObservedContentStatus::ContentAvailable)
        .then(|| body.to_string())
}

fn observed_step_entry(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let output = observed_step_body(step)?;
    if crate::finalizer::looks_like_planner_artifact(&output)
        || crate::finalizer::looks_like_internal_trace_artifact(&output)
    {
        return None;
    }
    Some(format!(
        "### {} skill({})\n{}",
        step.step_id,
        step.skill,
        trim_for_observed_prompt(&output, 1800)
    ))
}

fn observed_output_entries(loop_state: &LoopState) -> Vec<String> {
    let latest_listing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rfind(|(_, step)| {
            step.is_ok() && step.skill == "list_dir" && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx);
    let mut selected_indices = latest_listing_idx.into_iter().collect::<Vec<_>>();
    let mut recent_non_listing = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            step.is_ok() && step.skill != "list_dir" && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    if recent_non_listing.len() > 4 {
        recent_non_listing = recent_non_listing.split_off(recent_non_listing.len() - 4);
    }
    selected_indices.extend(recent_non_listing);
    selected_indices.sort_unstable();
    selected_indices.dedup();
    selected_indices
        .into_iter()
        .filter_map(|idx| observed_step_entry(&loop_state.executed_step_results[idx]))
        .collect()
}

pub(crate) fn has_observed_answer_candidates(loop_state: &LoopState) -> bool {
    !observed_output_entries(loop_state).is_empty()
}

fn observed_contract_json(agent_run_context: Option<&AgentRunContext>) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return "{}".to_string();
    };
    serde_json::json!({
        "routed_mode": route.routed_mode.as_str(),
        "response_shape": route.output_contract.response_shape.as_str(),
        "requires_content_evidence": route.output_contract.requires_content_evidence,
        "delivery_required": route.output_contract.delivery_required,
        "locator_kind": route.output_contract.locator_kind.as_str(),
        "delivery_intent": route.output_contract.delivery_intent.as_str(),
        "needs_clarify": route.needs_clarify,
    })
    .to_string()
}

fn resolved_user_intent(agent_run_context: Option<&AgentRunContext>, user_text: &str) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| user_text.trim())
        .to_string()
}

pub(crate) async fn synthesize_answer_from_observed_output(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let observed_entries = observed_output_entries(loop_state);
    if observed_entries.is_empty() {
        return None;
    }
    let observed_block = observed_entries.join("\n\n");
    let resolved_intent = resolved_user_intent(agent_run_context, user_text);
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH,
        OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_REQUEST__", user_text.trim()),
            ("__RESOLVED_USER_INTENT__", &resolved_intent),
            (
                "__OUTPUT_CONTRACT__",
                &observed_contract_json(agent_run_context),
            ),
            ("__OBSERVED_OUTPUTS__", &observed_block),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.command_intent.default_locale,
            ),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "observed_answer_fallback_prompt",
        &prompt_source,
        None,
    );
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .ok()?;
    let parsed_raw = serde_json::from_str::<ObservedAnswerFallbackOut>(llm_out.trim()).ok();
    let parsed = parsed_raw.or_else(|| {
        crate::extract_first_json_object_any(&llm_out)
            .and_then(|json| serde_json::from_str::<ObservedAnswerFallbackOut>(&json).ok())
    })?;
    let answer = parsed.answer.trim().to_string();
    let semantically_publishable = !answer.is_empty()
        && !parsed.needs_clarify
        && !crate::semantic_judge::is_meta_respond_instruction(state, task, &answer).await;
    let qualified = !answer.is_empty()
        && !parsed.needs_clarify
        && (parsed.qualified || semantically_publishable);
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(if qualified {
                crate::finalizer::FinalizerDisposition::QualifiedCompletion
            } else {
                crate::finalizer::FinalizerDisposition::AllowFallback
            }),
            parsed: true,
            contract_ok: qualified,
            completion_ok: Some(qualified),
            grounded_ok: Some(qualified),
            format_ok: Some(qualified),
            needs_clarify: Some(parsed.needs_clarify),
            confidence: Some(parsed.confidence.clamp(0.0, 1.0)),
            used_evidence_ids_count: observed_entries.len(),
            evidence_quotes_count: 0,
            ..Default::default()
        },
    ))
}

#[cfg(test)]
pub(crate) fn normalized_observed_listing(observed: &str) -> Option<String> {
    let lines = observed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::LoopState;
    use super::{
        extract_direct_scalar_from_generic_output, normalized_observed_listing,
        observed_output_entries,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    #[test]
    fn direct_scalar_ignores_exit_zero_prefix() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "git_basic", "exit=0\nmain\n"));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn direct_scalar_reads_extract_field_value_from_structured_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state).as_deref(),
            Some("rustclaw")
        );
    }

    #[test]
    fn direct_scalar_reports_missing_extract_field_as_field_absent() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state).as_deref(),
            Some("name 字段不存在")
        );
    }

    #[test]
    fn direct_scalar_reads_count_inventory_total_from_structured_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":12,"files":9,"dirs":3}}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state).as_deref(),
            Some("12")
        );
    }

    #[test]
    fn observed_entries_keep_latest_listing_plus_recent_non_listing_steps() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a.md\nb.md\nc.md\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_2", "read_file", "# A\nalpha\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_3", "read_file", "# B\nbeta\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_4", "read_file", "# C\ngamma\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_5", "read_file", "# D\ndelta\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_6", "read_file", "# E\nepsilon\n"));

        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 5);
        assert!(entries
            .iter()
            .any(|entry| entry.contains("step_1 skill(list_dir)")));
        assert!(entries
            .iter()
            .any(|entry| entry.contains("step_6 skill(read_file)")));
        assert!(!entries
            .iter()
            .any(|entry| entry.contains("step_2 skill(read_file)")));
    }

    #[test]
    fn normalized_listing_trims_blank_lines() {
        assert_eq!(
            normalized_observed_listing("\nfoo\n\nbar\n").as_deref(),
            Some("foo\nbar")
        );
    }

    #[test]
    fn observed_entries_use_read_range_excerpt_body_instead_of_raw_json() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","excerpt":"1|# RustClaw\n2|\n3|Hello"}"#,
        ));
        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("# RustClaw"));
        assert!(entries[0].contains("Hello"));
        assert!(!entries[0].contains(r#""action":"read_range""#));
    }

    #[test]
    fn observed_entries_compact_log_analyze_json_into_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "log_analyze",
            r#"{"path":"/tmp/test.log","total_lines":120,"keyword_counts":{"error":9,"panic":1},"recent_matches":["10: error one","20: panic two"]}"#,
        ));
        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("log_analyze path=/tmp/test.log total_lines=120"));
        assert!(entries[0].contains("keyword_counts: error=9, panic=1"));
        assert!(entries[0].contains("recent_matches:\n- 10: error one\n- 20: panic two"));
        assert!(!entries[0].contains(r#""keyword_counts""#));
    }
}
