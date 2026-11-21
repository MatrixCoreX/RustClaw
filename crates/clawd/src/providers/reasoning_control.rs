//! Provider protocol capabilities, independent of task semantics and total token budgets.
use super::client::ChatRequestHints;
use serde_json::{json, Map, Value};

pub(super) fn apply_openai_reasoning(
    body: &mut Map<String, Value>,
    model: &str,
    hints: &ChatRequestHints,
) {
    // M3 ignores reasoning_effort; its documented control is an adaptive/disabled toggle.
    // A structured verification call requests a direct verdict over supplied evidence.
    // Other M3 turns retain adaptive thinking; M3.1 must never receive disabled thinking.
    // https://platform.minimax.io/docs/api-reference/text-openai-api
    if model.eq_ignore_ascii_case("MiniMax-M3") {
        if hints.structured_verification {
            body.insert("thinking".into(), json!({"type":"disabled"}));
        }
        return;
    }
    if let Some(effort) = hints.reasoning_effort.as_deref() {
        body.insert("reasoning_effort".into(), json!(effort));
    }
}

#[cfg(test)]
#[path = "reasoning_control_tests.rs"]
mod tests;

/// Return true when the model owns a different protocol contract from Claude's manual budget.
pub(super) fn apply_anthropic_reasoning(
    body: &mut Map<String, Value>,
    model: &str,
    hints: &ChatRequestHints,
) -> bool {
    if model.eq_ignore_ascii_case("MiniMax-M3") {
        if hints.structured_verification {
            body.insert("thinking".into(), json!({"type":"disabled"}));
        }
        return true;
    }
    if model.eq_ignore_ascii_case("MiniMax-M3.1-Flash") {
        if let Some(effort) = hints.reasoning_effort.as_deref() {
            body.insert("output_config".into(), json!({"effort":effort}));
        }
        return true;
    }
    false
}
