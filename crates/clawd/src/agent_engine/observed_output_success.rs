use crate::{agent_engine::LoopState, AppState};

#[derive(Debug, Clone)]
pub(crate) struct GenericObservedOutput {
    pub(crate) skill: String,
    #[cfg(test)]
    pub(crate) action_label: String,
    pub(crate) body: String,
}

pub(super) fn latest_successful_step_index<F>(loop_state: &LoopState, predicate: F) -> Option<usize>
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

pub(super) fn has_successful_step_for_skill(loop_state: &LoopState, skill_name: &str) -> bool {
    latest_successful_step_index(loop_state, |step| step.skill == skill_name && step.is_ok())
        .is_some()
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
    extract_latest_generic_successful_output_with_state(None, loop_state)
}

pub(super) fn extract_latest_generic_successful_output_with_state(
    state: Option<&AppState>,
    loop_state: &LoopState,
) -> Option<GenericObservedOutput> {
    let idx = latest_successful_step_index(loop_state, |step| {
        if matches!(
            step.skill.as_str(),
            "read_file" | "list_dir" | "respond" | "synthesize_answer" | "think"
        ) {
            return false;
        }
        let raw_body = step.output.as_deref().map(str::trim).unwrap_or_default();
        let body = normalized_success_body_for_observed_output(raw_body);
        let body = body.trim();
        !body.is_empty()
            && (crate::finalize::classify_observed_content_status(body)
                == crate::finalize::ObservedContentStatus::ContentAvailable
                || super::structured_scalar_candidate(
                    state,
                    None,
                    &step.skill,
                    body,
                    None,
                    None,
                    false,
                    true,
                    false,
                )
                .is_some()
                || super::structured_observed_body(&step.skill, body).is_some())
            || super::market_quote_output_has_scalar_price(state, &step.skill, body)
            || super::system_basic_info_value(&step.skill, body).is_some()
            || super::system_basic_structured_doc_value(&step.skill, body).is_some()
            || super::system_basic_existence_with_path_value(&step.skill, body).is_some()
    })?;
    let step = &loop_state.executed_step_results[idx];
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    let body = normalized_success_body_for_observed_output(body);
    Some(GenericObservedOutput {
        skill: step.skill.clone(),
        #[cfg(test)]
        action_label: format!("{} skill({}): success", step.step_id, step.skill),
        body,
    })
}

pub(crate) fn normalized_success_body_for_observed_output(raw: &str) -> String {
    let trimmed = raw.trim();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return trimmed.to_string();
    };
    let Some(obj) = value.as_object() else {
        return trimmed.to_string();
    };
    if let Some(extra) = obj.get("extra").filter(|extra| {
        json_object_is_direct_observation_body(extra)
            || json_object_is_transform_observation_body(extra)
    }) {
        return extra.to_string();
    }
    trimmed.to_string()
}

fn json_object_is_transform_observation_body(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.contains_key("status")
        && (obj.contains_key("formatted")
            || obj.contains_key("output")
            || obj.contains_key("result"))
}

fn json_object_is_direct_observation_body(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    [
        "action",
        "field_value",
        "command_output",
        "clawd_process_count",
        "clawd_health_port_open",
        "system_health",
        "service_name",
        "manager_type",
        "requested_action",
        "executed_actions",
    ]
    .iter()
    .any(|field| obj.contains_key(*field))
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn extract_latest_successful_read_file_output_prefers_content_body() {
        let mut loop_state = LoopState::default();
        loop_state.executed_step_results.push(ok_step(
            "subtask#2",
            "read_file",
            "# Test Workspace\nThis directory is reserved.",
        ));
        let observed = extract_latest_successful_read_file_output(&loop_state);
        assert_eq!(
            observed.as_deref(),
            Some("# Test Workspace\nThis directory is reserved.")
        );
    }

    #[test]
    fn extract_latest_successful_list_dir_output_prefers_content_body() {
        let mut loop_state = LoopState::default();
        loop_state.executed_step_results.push(ok_step(
            "subtask#1",
            "list_dir",
            "file1.txt\nsubdir/\nfile2.md",
        ));
        let observed = extract_latest_successful_list_dir_output(&loop_state);
        assert_eq!(observed.as_deref(), Some("file1.txt\nsubdir/\nfile2.md"));
    }

    #[test]
    fn extract_latest_generic_successful_output_prefers_non_read_non_list_skill() {
        let mut loop_state = LoopState::default();
        loop_state
            .executed_step_results
            .push(ok_step("subtask#1", "read_file", "hello"));
        loop_state
            .executed_step_results
            .push(ok_step("subtask#2", "run_cmd", "testuser"));
        let observed =
            extract_latest_generic_successful_output(&loop_state).expect("generic observed output");
        assert!(observed.action_label.contains("skill(run_cmd): success"));
        assert_eq!(observed.body, "testuser");
    }

    #[test]
    fn extract_latest_generic_successful_output_skips_non_content() {
        let mut loop_state = LoopState::default();
        loop_state
            .executed_step_results
            .push(ok_step("subtask#1", "chat", "FILE:/tmp/demo.txt"));
        assert!(extract_latest_generic_successful_output(&loop_state).is_none());
    }
}
