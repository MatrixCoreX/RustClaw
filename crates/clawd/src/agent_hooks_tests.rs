use serde_json::json;

use super::{evaluate_pre_tool_use, structured_hook_error, HookPolicy};

#[test]
fn pre_tool_use_hook_allows_without_policy_match() {
    let policy = HookPolicy::default();
    let outcome = evaluate_pre_tool_use(&policy, "filesystem.list_entries");

    assert_eq!(outcome.stage, "pre_tool_use");
    assert_eq!(outcome.decision, "allow");
    assert_eq!(outcome.reason_code, "pre_tool_use_allowed");
    assert_eq!(outcome.action_ref, "filesystem.list_entries");
}

#[test]
fn pre_tool_use_hook_blocks_exact_machine_action_ref() {
    let policy = HookPolicy {
        blocked_action_refs: vec!["filesystem.remove_path".to_string()],
        ..HookPolicy::default()
    };
    let outcome = evaluate_pre_tool_use(&policy, "filesystem.remove_path");

    assert_eq!(outcome.decision, "deny");
    assert_eq!(outcome.reason_code, "pre_tool_use_blocked");
    let error = structured_hook_error(&outcome);
    let parsed: serde_json::Value = serde_json::from_str(&error).expect("structured hook error");
    assert_eq!(parsed["owner_layer"], json!("agent_hooks"));
    assert_eq!(parsed["decision"], json!("deny"));
    assert_eq!(parsed["action_ref"], json!("filesystem.remove_path"));
    assert_eq!(
        parsed["message_key"],
        json!("clawd.agent_hook.pre_tool_use_blocked")
    );
}

#[test]
fn pre_tool_use_hook_requires_confirmation_from_machine_action_ref() {
    let policy = HookPolicy {
        require_confirmation_action_refs: vec!["package.install".to_string()],
        ..HookPolicy::default()
    };
    let outcome = evaluate_pre_tool_use(&policy, "package.install");

    assert_eq!(outcome.decision, "require_confirmation");
    assert_eq!(outcome.reason_code, "pre_tool_use_requires_confirmation");
}

#[test]
fn pre_tool_use_hook_blocks_whole_tool_by_machine_token() {
    let policy = HookPolicy {
        blocked_tools: vec!["run_cmd".to_string()],
        ..HookPolicy::default()
    };
    let outcome = evaluate_pre_tool_use(&policy, "run_cmd");

    assert_eq!(outcome.decision, "deny");
}

#[test]
fn post_tool_use_hook_records_machine_status_reason() {
    let outcome =
        super::post_tool_use_outcome("video_generate", &json!({"action": "poll"}), "error");

    assert_eq!(outcome.stage, "post_tool_use");
    assert_eq!(outcome.decision, "allow");
    assert_eq!(outcome.reason_code, "post_tool_use_error");
    assert_eq!(outcome.action_ref, "video_generate.poll");
}

#[test]
fn stop_hook_records_final_status_reason() {
    let outcome = super::stop_outcome("resume_failure");

    assert_eq!(outcome.stage, "stop");
    assert_eq!(outcome.decision, "allow");
    assert_eq!(outcome.reason_code, "stop_resume_failure");
    assert_eq!(outcome.action_ref, "agent_loop.stop");
}

#[test]
fn session_and_prompt_hooks_use_machine_action_refs() {
    let session_start = super::session_start_outcome();
    let session_end = super::session_end_outcome("success");
    let prompt = super::user_prompt_submit_outcome();

    assert_eq!(session_start.stage, "session_start");
    assert_eq!(session_start.action_ref, "agent_loop.session_start");
    assert_eq!(session_end.reason_code, "session_end_success");
    assert_eq!(session_end.action_ref, "agent_loop.session_end");
    assert_eq!(prompt.stage, "user_prompt_submit");
    assert_eq!(prompt.action_ref, "agent_loop.user_prompt_submit");
}
