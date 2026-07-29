use serde_json::{json, Value};

use super::LoopState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentCheckpointStage {
    Planning,
    ToolExecution,
    Verification,
    PatchReview,
    FinalSynthesis,
}

impl AgentCheckpointStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::ToolExecution => "tool_execution",
            Self::Verification => "verification",
            Self::PatchReview => "patch_review",
            Self::FinalSynthesis => "final_synthesis",
        }
    }

    fn from_machine_token(value: &str) -> Option<Self> {
        match value.trim() {
            "planning" => Some(Self::Planning),
            "tool_execution" => Some(Self::ToolExecution),
            "verification" => Some(Self::Verification),
            "patch_review" => Some(Self::PatchReview),
            "final_synthesis" => Some(Self::FinalSynthesis),
            _ => None,
        }
    }
}

pub(crate) fn build_checkpoint_resume_state(
    loop_state: &LoopState,
    stage: AgentCheckpointStage,
) -> Value {
    let executed_step_provenance = crate::task_journal::checkpoint_step_provenance_records(
        &loop_state.round_traces,
        &loop_state.executed_step_results,
    );
    json!({
        "schema_version": 1,
        "stage": stage.as_str(),
        "loaded_capability_skills": loop_state
            .loaded_capability_skills
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        "loaded_mcp_capabilities": loop_state
            .loaded_mcp_capabilities
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        "active_capability_scopes": loop_state.active_capability_scopes,
        "last_output": loop_state.last_output,
        "history_compact": loop_state.history_compact,
        "task_observations": loop_state.task_observations,
        "executed_step_provenance": executed_step_provenance,
        "latest_validation_result": loop_state.latest_validation_result,
        "delivery_messages": loop_state.delivery_messages,
        "last_user_visible_respond": loop_state.last_user_visible_respond,
        "last_publishable_synthesis_output": loop_state.last_publishable_synthesis_output,
        "last_capability_synthesis_output": loop_state.last_capability_synthesis_output,
        "executed_step_results": loop_state
            .executed_step_results
            .iter()
            .map(|step| json!({
                "step_id": step.step_id,
                "skill": step.skill,
                "status": step.status.as_str(),
                "output": step.output,
                "error": step.error,
                "started_at": step.started_at,
                "finished_at": step.finished_at,
            }))
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn restore_checkpoint_resume_state(
    loop_state: &mut LoopState,
    boundary_context: &Value,
) -> AgentCheckpointStage {
    let Some(resume_state) = boundary_context
        .get("agent_loop_resume_state")
        .filter(|value| value.get("schema_version").and_then(Value::as_u64) == Some(1))
    else {
        return AgentCheckpointStage::Planning;
    };
    let stage = resume_state
        .get("stage")
        .and_then(Value::as_str)
        .and_then(AgentCheckpointStage::from_machine_token)
        .unwrap_or(AgentCheckpointStage::Planning);
    loop_state.output_vars.insert(
        "agent_loop.resume_stage".to_string(),
        stage.as_str().to_string(),
    );
    super::capability_discovery::restore_capability_scope_state(
        loop_state,
        string_array(resume_state, "active_capability_scopes"),
        string_array(resume_state, "loaded_capability_skills"),
        string_array(resume_state, "loaded_mcp_capabilities"),
    );

    if let Some(last_output) = string_field(resume_state, "last_output") {
        loop_state.last_output = Some(last_output.clone());
        loop_state
            .output_vars
            .insert("last_output".to_string(), last_output);
    }
    extend_unique_strings(
        &mut loop_state.history_compact,
        string_array(resume_state, "history_compact"),
    );
    extend_unique_values(
        &mut loop_state.task_observations,
        value_array(resume_state, "task_observations"),
    );
    extend_unique_values(
        &mut loop_state.task_observations,
        value_array(resume_state, "executed_step_provenance"),
    );
    loop_state.latest_validation_result = resume_state
        .get("latest_validation_result")
        .filter(|value| !value.is_null())
        .cloned();
    extend_unique_strings(
        &mut loop_state.delivery_messages,
        string_array(resume_state, "delivery_messages"),
    );
    loop_state.last_user_visible_respond = string_field(resume_state, "last_user_visible_respond");
    loop_state.last_publishable_synthesis_output =
        string_field(resume_state, "last_publishable_synthesis_output");
    loop_state.last_capability_synthesis_output =
        string_field(resume_state, "last_capability_synthesis_output");
    if loop_state.executed_step_results.is_empty() {
        loop_state
            .executed_step_results
            .extend(step_results(resume_state, "executed_step_results"));
    }
    stage
}

fn step_results(value: &Value, key: &str) -> Vec<crate::executor::StepExecutionResult> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let status = match item.get("status").and_then(Value::as_str)? {
                "ok" => crate::executor::StepExecutionStatus::Ok,
                "error" => crate::executor::StepExecutionStatus::Error,
                _ => return None,
            };
            Some(crate::executor::StepExecutionResult {
                step_id: item.get("step_id")?.as_str()?.to_string(),
                skill: item.get("skill")?.as_str()?.to_string(),
                status,
                output: optional_string(item, "output"),
                error: optional_string(item, "error"),
                started_at: item.get("started_at").and_then(Value::as_u64).unwrap_or(0),
                finished_at: item.get("finished_at").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn value_array(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .cloned()
        .collect()
}

fn extend_unique_strings(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}

fn extend_unique_values(target: &mut Vec<Value>, values: Vec<Value>) {
    for value in values {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}

#[cfg(test)]
#[path = "checkpoint_resume_state_tests.rs"]
mod tests;
