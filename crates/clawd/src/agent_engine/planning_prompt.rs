use super::{LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH, SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH};

pub(super) fn build_incremental_plan_prompt(
    prompt_template: &str,
    user_request: &str,
    goal: &str,
    turn_analysis: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    recent_assistant_replies: &str,
    request_language_hint: &str,
    config_response_language: &str,
    agent_runtime_identity: &str,
    round: usize,
    history_compact: &str,
    attempt_ledger: &str,
    last_round_output: &str,
    runtime_os: &str,
    runtime_shell: &str,
    workspace_root: &str,
) -> String {
    crate::render_prompt_template(
        prompt_template,
        &[
            ("__USER_REQUEST__", user_request),
            ("__GOAL__", goal),
            ("__TURN_ANALYSIS__", turn_analysis),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            ("__RECENT_ASSISTANT_REPLIES__", recent_assistant_replies),
            ("__REQUEST_LANGUAGE_HINT__", request_language_hint),
            ("__CONFIG_RESPONSE_LANGUAGE__", config_response_language),
            ("__AGENT_RUNTIME_IDENTITY__", agent_runtime_identity),
            ("__ROUND__", &round.to_string()),
            ("__HISTORY_COMPACT__", history_compact),
            ("__ATTEMPT_LEDGER__", attempt_ledger),
            ("__LAST_ROUND_OUTPUT__", last_round_output),
            ("__RUNTIME_OS__", runtime_os),
            ("__RUNTIME_SHELL__", runtime_shell),
            ("__WORKSPACE_ROOT__", workspace_root),
        ],
    )
}

pub(super) fn runtime_os_label() -> String {
    format!(
        "{} (family={}, arch={})",
        std::env::consts::OS,
        std::env::consts::FAMILY,
        std::env::consts::ARCH
    )
}

pub(super) fn runtime_shell_label() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("COMSPEC")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "(unknown shell)".to_string())
}

pub(super) fn round1_prompt_spec() -> (&'static str, &'static str) {
    (
        "single_plan_execution_prompt",
        SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH,
    )
}

pub(super) fn incremental_prompt_spec() -> (&'static str, &'static str) {
    (
        "loop_incremental_plan_prompt",
        LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH,
    )
}

#[cfg(test)]
#[path = "planning_prompt_tests.rs"]
mod tests;
