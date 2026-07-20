use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::{
    classify_skill_failure_recovery, strip_internal_execution_args,
    strip_unsupported_planner_metadata_args, synthesize_answer_allows_direct_fallback,
    synthesize_bounded_read_range_direct_answer,
    synthesize_direct_fallback_would_passthrough_multiline_read_range,
    synthesize_direct_observed_fallback_answer,
    synthesize_evidence_policy_direct_observed_fallback_answer, synthesize_failure_observed_facts,
    synthesize_failure_should_replan, synthesize_route_allows_direct_fallback,
    unresolved_file_token_delivery_artifact,
};
use crate::agent_engine::{AgentRunContext, LoopState};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    AgentAction, AgentRuntimeConfig, AppState, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;

#[path = "dispatch_support_tests/active_recipe_terminal_discussion.rs"]
mod active_recipe_terminal_discussion;
#[path = "dispatch_support_tests/machine_envelope.rs"]
mod machine_envelope;
#[path = "dispatch_support_tests/read_range_synthesis_fallback.rs"]
mod read_range_synthesis_fallback;
#[path = "dispatch_support_tests/respond_template_guard.rs"]
mod respond_template_guard;
#[path = "dispatch_support_tests/scalar_config_fallback.rs"]
mod scalar_config_fallback;
#[path = "dispatch_support_tests/synthesize_failure_replan.rs"]
mod synthesize_failure_replan;
#[path = "dispatch_support_tests/text_protocol_boundary.rs"]
mod text_protocol_boundary;

fn test_state_with_registry() -> AppState {
    test_state_with_registry_excluding(&[])
}

fn test_state_with_registry_excluding(disabled: &[&str]) -> AppState {
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    let registry_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .filter(|skill| !disabled.iter().any(|disabled| skill.as_str() == *disabled))
        .collect::<HashSet<_>>();
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: Some(Arc::new(registry)),
                skills_list: Arc::new(enabled),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

#[test]
fn unresolved_disabled_capability_error_is_machine_payload() {
    let state = test_state_with_registry_excluding(&["fs_basic"]);
    let error = super::unresolved_capability_error(
        &state,
        "filesystem.list_entries",
        &serde_json::json!({"path": "."}),
    );
    let payload: serde_json::Value =
        serde_json::from_str(&error).expect("unresolved capability error json");

    assert_eq!(payload["error_kind"], "capability_disabled");
    assert_eq!(payload["message_key"], "capability_disabled");
    assert_eq!(payload["owner_layer"], "capability_resolver");
    assert_eq!(payload["outcome"], "blocked");
    assert_eq!(payload["source"], "registry");
    assert_eq!(payload["capability_ref"], "filesystem.list_entries");
    assert_eq!(payload["resolved_ref"], "tool:fs_basic");
    assert_eq!(payload["planner_kind"], "tool");
}

#[test]
fn retryable_run_cmd_failure_stops_before_remaining_tool_action() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"resume_fail_cmd_001_xyz"}),
        },
        AgentAction::CallSkill {
            skill: "stock".to_string(),
            args: serde_json::json!({"symbol":"ETH"}),
        },
    ];

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            8,
            "run_cmd",
            Some(&serde_json::json!({"command":"resume_fail_cmd_001_xyz"})),
            "command not found",
        ),
        Some("recoverable_failure_continue_round")
    );
}

#[test]
fn literal_run_cmd_failure_before_remaining_action_finalizes_without_replan() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "echo before",
                "_clawd_literal_command": true
            }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "missing_literal_cmd_for_stop",
                "_clawd_literal_command": true
            }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "echo after",
                "_clawd_literal_command": true
            }),
        },
    ];

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            1,
            8,
            "run_cmd",
            Some(&serde_json::json!({
                "command": "missing_literal_cmd_for_stop",
                "_clawd_literal_command": true
            })),
            "command not found",
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn split_sequence_run_cmd_failure_continues_to_remaining_run_cmd() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "echo before",
                "_clawd_continue_on_error": true
            }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "missing_cmd_from_split",
                "_clawd_continue_on_error": true
            }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "echo after",
                "_clawd_continue_on_error": true
            }),
        },
    ];

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            1,
            8,
            "run_cmd",
            Some(&serde_json::json!({
                "command": "missing_cmd_from_split",
                "_clawd_continue_on_error": true
            })),
            "command not found",
        ),
        Some("recoverable_failure_continue_in_round")
    );
}

#[test]
fn internal_execution_args_are_removed_before_skill_call() {
    let mut args = serde_json::json!({
        "command": "echo visible",
        "_clawd_continue_on_error": true
    });

    strip_internal_execution_args(&mut args);

    assert_eq!(args, serde_json::json!({"command": "echo visible"}));
}

#[test]
fn unsupported_confirm_arg_is_removed_before_make_dir_skill_call() {
    let state = test_state_with_registry();
    let canonical = state.resolve_canonical_skill_name("make_dir");
    let manifest = state.skill_manifest(&canonical).expect("make_dir manifest");
    assert!(
        manifest.input_schema.is_some(),
        "make_dir manifest should expose input_schema"
    );
    let mut args = serde_json::json!({
        "path": "document",
        "confirm": true
    });

    let removed = strip_unsupported_planner_metadata_args(&state, &canonical, &mut args);

    assert_eq!(removed, vec!["confirm"]);
    assert_eq!(args, serde_json::json!({"path": "document"}));
}

#[test]
fn supported_parents_arg_is_kept_before_make_dir_skill_call() {
    let state = test_state_with_registry();
    let canonical = state.resolve_canonical_skill_name("make_dir");
    let manifest = state.skill_manifest(&canonical).expect("make_dir manifest");
    assert!(
        manifest
            .input_schema
            .as_ref()
            .and_then(|schema| schema.get("properties"))
            .and_then(|properties| properties.get("parents"))
            .is_some(),
        "make_dir manifest should expose parents input_schema property"
    );
    let mut args = serde_json::json!({
        "path": "document",
        "parents": true
    });

    let removed = strip_unsupported_planner_metadata_args(&state, &canonical, &mut args);

    assert!(removed.is_empty(), "{removed:?}");
    assert_eq!(
        args,
        serde_json::json!({"path": "document", "parents": true})
    );
}

#[test]
fn confirm_arg_is_kept_when_skill_schema_declares_it() {
    let state = test_state_with_registry();
    let mut args = serde_json::json!({
        "action": "register_external_skill",
        "skill_name": "demo_skill",
        "confirm": true
    });

    let removed = strip_unsupported_planner_metadata_args(&state, "extension_manager", &mut args);

    assert!(removed.is_empty(), "{removed:?}");
    assert_eq!(
        args,
        serde_json::json!({
            "action": "register_external_skill",
            "skill_name": "demo_skill",
            "confirm": true
        })
    );
}

#[test]
fn invalid_file_delivery_token_detects_embedded_runtime_observation() {
    let candidate = r#"FILE:/tmp/docs/{"action":"inventory_dir","counts":{"files":2},"names":["a.txt","b.txt"]}"#;
    let compound_delivery = "FILE:/tmp/docs/a.txt\n\n[app]\nname = \"RustClaw NL Fixture\"";

    assert!(unresolved_file_token_delivery_artifact(candidate));
    assert!(unresolved_file_token_delivery_artifact(
        "FILE:{{last_output}}"
    ));
    assert!(!unresolved_file_token_delivery_artifact(compound_delivery));
    assert!(!unresolved_file_token_delivery_artifact(
        "FILE:/tmp/docs/a.txt"
    ));
    assert!(!unresolved_file_token_delivery_artifact(
        "请查看 /tmp/docs/a.txt"
    ));
}

#[test]
fn failure_at_round_cap_with_terminal_discussion_remaining_finalizes() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({"path":"logs"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"definitely_missing_command"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            1,
            2,
            "run_cmd",
            Some(&serde_json::json!({"command":"definitely_missing_command"})),
            "command not found",
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn terminal_run_cmd_failure_after_prior_command_finalizes_for_summary() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"echo READY"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"definitely_missing_command"}),
        },
    ];

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            1,
            4,
            "run_cmd",
            Some(&serde_json::json!({"command":"definitely_missing_command"})),
            "command not found",
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn single_literal_structured_run_cmd_failure_finalizes_as_observed_result() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({"command":"printf problem >&2; exit 7"}),
    }];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 7\nstderr:\nproblem",
            "extra": {
                "command": "printf problem >&2; exit 7",
                "exit_code": 7,
                "stderr": "problem",
                "output_truncated": false
            }
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "run_cmd",
            Some(&serde_json::json!({
                "command":"printf problem >&2; exit 7",
                "_clawd_literal_command": true
            })),
            &err,
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn permission_failure_without_remaining_action_finalizes_without_shell_fallback() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action":"read_range","path":"/root/secret.txt"}),
    }];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "system_basic",
            "error_kind": "permission_denied",
            "error_text": "permission denied: /root/secret.txt"
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "system_basic",
            Some(&serde_json::json!({"action":"read_range","path":"/root/secret.txt"})),
            &err,
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn crypto_account_access_failure_finalizes_without_replan() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "crypto".to_string(),
            args: serde_json::json!({"action":"positions"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let marker = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "crypto",
            "error_kind": "unknown",
            "error_text": marker,
            "extra": null
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            2,
            "crypto",
            Some(&serde_json::json!({"action":"positions"})),
            &err,
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn fs_basic_virtual_permission_failure_finalizes_without_shell_fallback() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args: serde_json::json!({
                "action":"read_text_range",
                "path":"/root/secret.txt"
            }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"head -n 1 /root/secret.txt"}),
        },
    ];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "system_basic",
            "error_kind": "permission_denied",
            "error_text": "permission denied: /root/secret.txt"
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "fs_basic",
            Some(&serde_json::json!({
                "action":"read_text_range",
                "path":"/root/secret.txt"
            })),
            &err,
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn explicit_missing_target_without_fallback_finalizes_not_found() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action":"read_range","path":"missing.md"}),
    }];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "system_basic",
            "error_kind": "not_found",
            "error_text": "path not found: missing.md"
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "system_basic",
            Some(&serde_json::json!({"action":"read_range","path":"missing.md"})),
            &err,
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn repairable_missing_target_continues_next_round() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: serde_json::json!({
            "path":"missing.md",
            "_clawd_missing_target_repairable": true
        }),
    }];
    let err = "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.md";

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "read_file",
            Some(&serde_json::json!({
                "path":"missing.md",
                "_clawd_missing_target_repairable": true
            })),
            err,
        ),
        Some("recoverable_failure_continue_round")
    );
}

#[test]
fn planner_protocol_failure_replans_next_round() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action":"check_exists","path":"README.md"}),
    }];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "system_basic",
            "error_kind": "unsupported_action",
            "error_text": "unknown action: check_exists"
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "system_basic",
            Some(&serde_json::json!({"action":"check_exists","path":"README.md"})),
            &err,
        ),
        Some("recoverable_failure_continue_round")
    );
}

#[test]
fn planner_generated_terminal_command_failure_replans_but_literal_command_finalizes() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({"command":"missing_tool --version"}),
    }];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "extra": {
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "missing_tool: command not found"
            }
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "run_cmd",
            Some(&serde_json::json!({"command":"missing_tool --version"})),
            &err,
        ),
        Some("recoverable_failure_continue_round")
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "run_cmd",
            Some(&serde_json::json!({
                "command":"missing_tool --version",
                "_clawd_literal_command": true
            })),
            &err,
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn literal_run_cmd_failure_before_discussion_only_tail_finalizes() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command":"missing_tool --version",
                "_clawd_literal_command": true
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "extra": {
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "missing_tool: command not found"
            }
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "run_cmd",
            Some(&serde_json::json!({
                "command":"missing_tool --version",
                "_clawd_literal_command": true
            })),
            &err,
        ),
        Some("recoverable_failure_finalize")
    );
}

#[test]
fn literal_command_failure_with_structured_repairable_marker_replans() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({
            "command":"missing_tool --version",
            "_clawd_literal_command": true,
            "_clawd_literal_failure_repairable": true
        }),
    }];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "extra": {
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "missing_tool: command not found"
            }
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "run_cmd",
            Some(&serde_json::json!({
                "command":"missing_tool --version",
                "_clawd_literal_command": true,
                "_clawd_literal_failure_repairable": true
            })),
            &err,
        ),
        Some("recoverable_failure_continue_round")
    );
}

#[test]
fn visible_run_cmd_error_without_structured_payload_replans() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({"command":"missing_tool --version"}),
    }];
    let err = "command failed: command not found (exit code 127); stderr: missing_tool: command not found";

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "run_cmd",
            Some(&serde_json::json!({"command":"missing_tool --version"})),
            err,
        ),
        Some("recoverable_failure_continue_round")
    );
}

#[test]
fn planner_generated_command_failure_replans_before_discussion_only_tail() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"missing_tool --version"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "extra": {
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "missing_tool: command not found"
            }
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "run_cmd",
            Some(&serde_json::json!({"command":"missing_tool --version"})),
            &err,
        ),
        Some("recoverable_failure_continue_round")
    );
}

#[test]
fn recoverable_nonterminal_failure_with_only_discussion_remaining_continues_next_round() {
    let state = test_state_with_registry();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({"path":"missing_dir"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "list_dir",
            "error_kind": "ambiguous_target",
            "error_text": "directory locator matched multiple candidates",
            "extra": { "candidates": ["/tmp/a", "/tmp/b"] }
        })
    );

    assert_eq!(
        classify_skill_failure_recovery(
            &state,
            &actions,
            0,
            4,
            "list_dir",
            Some(&serde_json::json!({"path":"missing_dir"})),
            &err,
        ),
        Some("recoverable_failure_continue_round")
    );
}

#[test]
fn terminal_direct_respond_publishes_even_when_last_output_matches() {
    let state = test_state_with_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-terminal-direct-respond".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: String::new(),
    };
    let policy = crate::agent_engine::support::load_agent_loop_guard_policy(&state);
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    let content = r#"{"cancel_ref":"dry-run","adapter_kind":"local_process_poll","status":"cancelled","terminal_projection":{"state":"cancelled"}}"#;
    loop_state.last_output = Some(content.to_string());
    let actions = vec![AgentAction::Respond {
        content: content.to_string(),
    }];

    let outcome = super::handle_respond_action(
        &state,
        &task,
        &actions,
        &mut loop_state,
        &policy,
        0,
        1,
        1,
        "respond:terminal_direct",
        content,
        None,
    );

    assert!(outcome.should_stop);
    assert_eq!(outcome.stop_signal.as_deref(), Some("respond"));
    assert!(outcome.ended_with_user_visible_output);
    assert_eq!(loop_state.delivery_messages, vec![content.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(content)
    );
}

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
fn synthesize_answer_direct_fallback_only_for_single_last_output() {
    assert!(synthesize_answer_allows_direct_fallback(&[]));
    assert!(synthesize_answer_allows_direct_fallback(&[
        "last_output".to_string()
    ]));
    assert!(!synthesize_answer_allows_direct_fallback(&[
        "s1".to_string(),
        "s2".to_string()
    ]));
    assert!(!synthesize_answer_allows_direct_fallback(&[
        "last_output".to_string(),
        "step_1".to_string()
    ]));
}

#[test]
fn synthesize_direct_fallback_uses_scalar_path_observation() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","facts":[{"path":".","resolved_path":"/home/guagua/rustclaw","exists":true}]}"#,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract {
            structured_field_selector: Some("resolved_path".to_string()),
            ..Default::default()
        },
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    let answer = synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx))
        .expect("scalar path fallback");

    assert_eq!(answer, "/home/guagua/rustclaw");
}

#[test]
fn contract_matrix_synthesis_defers_multiple_count_observations_to_model() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","path":"docs","recursive":false,"counts":{"total":3,"files":2,"dirs":1,"total_size_bytes":425}},"text":"{\"action\":\"count_inventory\",\"path\":\"docs\",\"recursive\":false,\"counts\":{\"total\":3,\"files\":2,\"dirs\":1,\"total_size_bytes\":425}}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","path":"logs","recursive":false,"counts":{"total":2,"files":2,"dirs":0,"total_size_bytes":2698}},"text":"{\"action\":\"count_inventory\",\"path\":\"logs\",\"recursive\":false,\"counts\":{\"total\":2,\"files\":2,\"dirs\":0,\"total_size_bytes\":2698}}"}"#,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "docs | logs".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(synthesize_evidence_policy_direct_observed_fallback_answer(
        &state,
        &loop_state,
        Some(&ctx)
    )
    .is_none());
}

#[test]
fn synthesize_direct_fallback_defers_multiple_count_observations_to_model() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","path":"docs","recursive":false,"counts":{"total":3,"files":2,"dirs":1,"total_size_bytes":425}},"text":"{\"action\":\"count_inventory\",\"path\":\"docs\",\"recursive\":false,\"counts\":{\"total\":3,\"files\":2,\"dirs\":1,\"total_size_bytes\":425}}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","path":"logs","recursive":false,"counts":{"total":2,"files":2,"dirs":0,"total_size_bytes":2698}},"text":"{\"action\":\"count_inventory\",\"path\":\"logs\",\"recursive\":false,\"counts\":{\"total\":2,\"files\":2,\"dirs\":0,\"total_size_bytes\":2698}}"}"#,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "docs | logs".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx)).is_none());
}

#[test]
fn synthesize_direct_fallback_defers_multi_observation_grounded_summary_to_model() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"dirs":["UI","crates"],"files":["README.md"]},"path":"/repo","resolved_path":"/repo"},"text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":2,\"files\":1,\"total\":3},\"names_by_kind\":{\"dirs\":[\"UI\",\"crates\"],\"files\":[\"README.md\"]},\"path\":\"/repo\",\"resolved_path\":\"/repo\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "config_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"name","path":"/repo/UI/package.json","resolved_path":"/repo/UI/package.json","value":"react-example","value_text":"react-example","value_type":"string"},"text":"{\"action\":\"extract_field\",\"exists\":true,\"field_path\":\"name\",\"path\":\"/repo/UI/package.json\",\"resolved_path\":\"/repo/UI/package.json\",\"value\":\"react-example\",\"value_text\":\"react-example\",\"value_type\":\"string\"}"}"#,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(synthesize_evidence_policy_direct_observed_fallback_answer(
        &state,
        &loop_state,
        Some(&ctx)
    )
    .is_none());
    assert!(synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx)).is_none());
}

#[test]
fn contract_matrix_synthesis_defers_multiline_content_excerpt_summary_to_model() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"7|{\"status\":\"ok\",\"response\":\"path resolved\"}\n8|{\"status\":\"ok\",\"response\":\"db inspected\"}\n9|{\"status\":\"ok\",\"response\":\"log tailed\"}\n10|{\"status\":\"ok\",\"response\":\"binding remembered\"}","path":"/tmp/model_io.log"}"#,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
        locator_hint: "/tmp/model_io.log".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(
        synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );
    assert!(!synthesize_route_allows_direct_fallback(Some(&ctx)));
    assert!(synthesize_evidence_policy_direct_observed_fallback_answer(
        &state,
        &loop_state,
        Some(&ctx)
    )
    .is_none());
}

#[test]
fn unclassified_strict_evidence_contract_defers_direct_fallback_to_synthesis() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "/home/guagua/rustclaw\n"));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(!synthesize_route_allows_direct_fallback(Some(&ctx)));
    assert!(synthesize_evidence_policy_direct_observed_fallback_answer(
        &state,
        &loop_state,
        Some(&ctx)
    )
    .is_none());
    assert!(synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx)).is_none());
}

#[test]
fn synthesize_route_allows_direct_fallback_for_plain_act_observed_read() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Filename,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "README.md".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
}

#[test]
fn synthesize_route_allows_direct_fallback_for_structured_listing_contract() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "document".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
}

#[test]
fn synthesize_route_allows_observed_fallback_for_unclassified_delivery() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
}

#[test]
fn synthesize_route_blocks_direct_fallback_for_unclassified_strict_evidence() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "logs".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(!synthesize_route_allows_direct_fallback(Some(&ctx)));
}

#[test]
fn synthesize_route_uses_llm_for_strict_raw_output_contract() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(!synthesize_route_allows_direct_fallback(Some(&ctx)));
}

#[test]
fn strict_raw_tail_read_uses_direct_observed_fallback_before_composer() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"98|WARN provider failed: http 401: Please carry the API secret key\n99|WARN memory preference fallback failed: http 401","path":"/tmp/clawd-dev.log"}"#,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
        locator_hint: "/tmp/clawd-dev.log".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(!synthesize_route_allows_direct_fallback(Some(&ctx)));
    assert_eq!(
        synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx)).as_deref(),
        Some(
            "WARN provider failed: http 401: Please carry the API secret key\nWARN memory preference fallback failed: http 401"
        )
    );
}
