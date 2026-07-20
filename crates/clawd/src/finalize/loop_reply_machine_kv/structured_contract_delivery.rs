use serde_json::Value;

use crate::agent_engine::LoopState;

pub(super) fn current_delivery_contains_full_structured_contract(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    delivery_messages
        .iter()
        .chain(loop_state.delivery_messages.iter())
        .chain(loop_state.last_user_visible_respond.iter())
        .any(|message| structured_contract_json_should_remain_full(message))
}

fn structured_contract_json_should_remain_full(text: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    object.contains_key("contract_marker")
        && (object.contains_key("async_timeout_policy")
            || object.contains_key("adapter_result")
            || object.contains_key("pending_async_job_contract")
            || object.contains_key("task_lifecycle")
            || object.contains_key("execution_policy"))
}
