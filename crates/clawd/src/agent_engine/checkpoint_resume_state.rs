use serde_json::{json, Value};

use super::LoopState;

const MAX_LAST_OUTPUT_CHARS: usize = 8_192;
const MAX_HISTORY_ITEMS: usize = 24;
const MAX_HISTORY_ITEM_CHARS: usize = 2_048;
const MAX_OBSERVATION_ITEMS: usize = 24;
const MAX_OBSERVATION_BYTES: usize = 8_192;
const MAX_VALIDATION_BYTES: usize = 16_384;
const MAX_DELIVERY_ITEMS: usize = 4;
const MAX_DELIVERY_ITEM_CHARS: usize = 8_192;

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
    let history_compact = bounded_recent_strings(
        &loop_state.history_compact,
        MAX_HISTORY_ITEMS,
        MAX_HISTORY_ITEM_CHARS,
    );
    let task_observations =
        bounded_recent_values(&loop_state.task_observations, MAX_OBSERVATION_ITEMS);
    let delivery_messages = bounded_recent_strings(
        &loop_state.delivery_messages,
        MAX_DELIVERY_ITEMS,
        MAX_DELIVERY_ITEM_CHARS,
    );
    let latest_validation_result = loop_state
        .latest_validation_result
        .as_ref()
        .filter(|value| bounded_json_value(value, MAX_VALIDATION_BYTES))
        .cloned();

    json!({
        "schema_version": 1,
        "stage": stage.as_str(),
        "loaded_capability_skills": loop_state
            .loaded_capability_skills
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        "last_output": loop_state
            .last_output
            .as_deref()
            .map(|value| bounded_chars(value, MAX_LAST_OUTPUT_CHARS)),
        "history_compact": history_compact,
        "task_observations": task_observations,
        "latest_validation_result": latest_validation_result,
        "delivery_messages": delivery_messages,
        "last_user_visible_respond": loop_state
            .last_user_visible_respond
            .as_deref()
            .map(|value| bounded_chars(value, MAX_DELIVERY_ITEM_CHARS)),
        "last_publishable_synthesis_output": loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .map(|value| bounded_chars(value, MAX_DELIVERY_ITEM_CHARS)),
        "last_capability_synthesis_output": loop_state
            .last_capability_synthesis_output
            .as_deref()
            .map(|value| bounded_chars(value, MAX_DELIVERY_ITEM_CHARS)),
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
    loop_state.loaded_capability_skills.extend(
        bounded_string_array(resume_state, "loaded_capability_skills")
            .into_iter()
            .filter(|token| super::capability_discovery::is_capability_group_token(token))
            .take(super::capability_discovery::MAX_LOADED_GROUPS_PER_TASK),
    );

    if let Some(last_output) = bounded_string_field(resume_state, "last_output") {
        loop_state.last_output = Some(last_output.clone());
        loop_state
            .output_vars
            .insert("last_output".to_string(), last_output);
    }
    extend_unique_strings(
        &mut loop_state.history_compact,
        bounded_string_array(resume_state, "history_compact"),
    );
    extend_unique_values(
        &mut loop_state.task_observations,
        bounded_value_array(resume_state, "task_observations"),
    );
    loop_state.latest_validation_result = resume_state
        .get("latest_validation_result")
        .filter(|value| !value.is_null() && bounded_json_value(value, MAX_VALIDATION_BYTES))
        .cloned();
    extend_unique_strings(
        &mut loop_state.delivery_messages,
        bounded_string_array(resume_state, "delivery_messages"),
    );
    loop_state.last_user_visible_respond =
        bounded_string_field(resume_state, "last_user_visible_respond");
    loop_state.last_publishable_synthesis_output =
        bounded_string_field(resume_state, "last_publishable_synthesis_output");
    loop_state.last_capability_synthesis_output =
        bounded_string_field(resume_state, "last_capability_synthesis_output");
    stage
}

fn bounded_recent_strings(values: &[String], limit: usize, char_limit: usize) -> Vec<String> {
    let start = values.len().saturating_sub(limit);
    values[start..]
        .iter()
        .map(|value| bounded_chars(value, char_limit))
        .collect()
}

fn bounded_recent_values(values: &[Value], limit: usize) -> Vec<Value> {
    let start = values.len().saturating_sub(limit);
    values[start..]
        .iter()
        .filter(|value| bounded_json_value(value, MAX_OBSERVATION_BYTES))
        .cloned()
        .collect()
}

fn bounded_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn bounded_json_value(value: &Value, byte_limit: usize) -> bool {
    serde_json::to_vec(value)
        .map(|encoded| encoded.len() <= byte_limit)
        .unwrap_or(false)
}

fn bounded_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| bounded_chars(item, MAX_DELIVERY_ITEM_CHARS))
}

fn bounded_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|item| bounded_chars(item, MAX_DELIVERY_ITEM_CHARS))
        .collect()
}

fn bounded_value_array(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| bounded_json_value(item, MAX_OBSERVATION_BYTES))
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
