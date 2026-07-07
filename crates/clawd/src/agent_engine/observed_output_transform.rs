use crate::{agent_engine::LoopState, AppState};

use super::{
    evidence_policy_checked_direct_candidate, normalized_success_body_for_observed_output,
};

pub(crate) fn transform_skill_formatted_output_candidate(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    transform_skill_formatted_output_value_candidate(&value).or_else(|| {
        value
            .get("extra")
            .and_then(transform_skill_formatted_output_value_candidate)
    })
}

fn transform_skill_formatted_output_value_candidate(value: &serde_json::Value) -> Option<String> {
    let status_ok = value
        .get("status")
        .and_then(|value| value.as_str())
        .map(|status| status.eq_ignore_ascii_case("ok"))
        .unwrap_or(false);
    if !status_ok {
        return None;
    }
    value
        .get("formatted")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|formatted| !formatted.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            value
                .get("output")
                .filter(|output| !output.is_null())
                .and_then(|output| serde_json::to_string(output).ok())
        })
        .or_else(|| {
            value
                .get("result")
                .filter(|result| !result.is_null())
                .and_then(|result| serde_json::to_string(result).ok())
        })
}

fn normalize_step_output_evidence_ref(reference: &str) -> Option<String> {
    let mut token = reference.trim().to_ascii_lowercase();
    if token.is_empty() || token == "last_output" {
        return None;
    }
    for suffix in [".output", "_output", ".text", "_text"] {
        if let Some(stripped) = token.strip_suffix(suffix) {
            token = stripped.to_string();
            break;
        }
    }
    if let Some(digits) = token.strip_prefix("step_") {
        return digits
            .chars()
            .all(|ch| ch.is_ascii_digit())
            .then(|| format!("step_{digits}"));
    }
    if let Some(digits) = token.strip_prefix("step") {
        return digits
            .chars()
            .all(|ch| ch.is_ascii_digit())
            .then(|| format!("step_{digits}"));
    }
    if let Some(digits) = token.strip_prefix('s') {
        return digits
            .chars()
            .all(|ch| ch.is_ascii_digit())
            .then(|| format!("step_{digits}"));
    }
    None
}

pub(crate) fn direct_answer_from_referenced_observation_i18n(
    loop_state: &LoopState,
    _state: &AppState,
    agent_run_context: Option<&super::AgentRunContext>,
    evidence_refs: &[String],
) -> Option<String> {
    let mut step_ids = Vec::new();
    for reference in evidence_refs {
        let Some(step_id) = normalize_step_output_evidence_ref(reference) else {
            continue;
        };
        if !step_ids.iter().any(|existing| existing == &step_id) {
            step_ids.push(step_id);
        }
    }
    let [step_id] = step_ids.as_slice() else {
        return None;
    };
    let step = loop_state.executed_step_results.iter().find(|step| {
        step.is_ok()
            && step.step_id == *step_id
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
    })?;
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())?;
    let body = normalized_success_body_for_observed_output(body);
    let answer = match step.skill.as_str() {
        "transform" => transform_skill_formatted_output_candidate(&body),
        _ => None,
    }?;
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    evidence_policy_checked_direct_candidate(route, loop_state, auto_locator_path, answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_engine::LoopState;
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
    fn transform_output_candidate_falls_back_to_result_json() {
        assert_eq!(
            transform_skill_formatted_output_candidate(
                r#"{"status":"ok","formatted":null,"result":[{"name":"beta"},{"name":"alpha"}]}"#
            )
            .as_deref(),
            Some(r#"[{"name":"beta"},{"name":"alpha"}]"#)
        );
    }

    #[test]
    fn transform_output_candidate_ignores_visible_text_json_payload() {
        let body = serde_json::json!({
            "status": "ok",
            "extra": {"action": "transform_data"},
            "text": serde_json::json!({"status": "ok", "formatted": "machine_value"}).to_string()
        })
        .to_string();

        assert!(transform_skill_formatted_output_candidate(&body).is_none());
    }

    #[test]
    fn referenced_transform_step_answer_ignores_earlier_directory_observation() {
        let state = AppState::test_default_with_fixture_provider();
        let mut loop_state = LoopState::new(3);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"extra":{"action":"tree_summary"},"text":"{\"action\":\"tree_summary\",\"tree\":{\"path\":\".\",\"child_count\":80}}"}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "transform",
            r#"{"extra":{"action":"transform_data","error":null,"formatted":"| name | score |\n| --- | --- |\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |\n","output":"| name | score |\n| --- | --- |\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |\n","result":[{"name":"beta","score":12},{"name":"gamma","score":9},{"name":"alpha","score":7}],"status":"ok"}}"#,
        ));

        let answer = direct_answer_from_referenced_observation_i18n(
            &loop_state,
            &state,
            None,
            &[String::from("s2.output")],
        )
        .expect("referenced transform step should produce formatted output");

        assert_eq!(
            answer,
            "| name | score |\n| --- | --- |\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |"
        );
        assert!(!answer.contains("tree_summary"));
    }
}
