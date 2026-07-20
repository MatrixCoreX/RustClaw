use claw_core::capability_result::{
    CapabilityDelivery, CapabilityDeliveryIntent, CapabilityResultEnvelope,
};
use serde_json::json;

use super::{bounded_result, eligible_for_capability_result_synthesis, MAX_RESULT_JSON_CHARS};
use crate::agent_engine::{AgentRunContext, LoopState};

#[test]
fn ordinary_free_response_uses_generic_synthesis() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "filesystem.list",
            Some("list".to_string()),
            json!({"entries": ["README.md"]}),
        ));
    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn config_mutation_receipt_uses_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "config_edit",
            Some("apply_config_change".to_string()),
            json!({
                "extra": {
                    "path": "configs/config.toml",
                    "field_path": "skills.skill_switches.example",
                    "old_value": null,
                    "new_value": true,
                    "applied": true,
                    "validated": true
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn docker_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    for (action, data) in [
        (
            "ps",
            json!({"extra": {"action": "ps", "exit_code": 0, "output": "container-a"}}),
        ),
        (
            "logs",
            json!({"extra": {"action": "logs", "exit_code": 0, "output": "ready"}}),
        ),
        (
            "restart",
            json!({"extra": {"action": "restart", "exit_code": 0, "output": "container-a"}}),
        ),
    ] {
        loop_state
            .capability_results
            .push(CapabilityResultEnvelope::ok(
                "docker_basic",
                Some(action.to_string()),
                data,
            ));
    }

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn database_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    for (action, data) in [
        (
            "list_tables",
            json!({
                "extra": {
                    "action": "list_tables",
                    "table_count": 2,
                    "tables": ["orders", "users"]
                }
            }),
        ),
        (
            "schema_version",
            json!({
                "extra": {
                    "action": "schema_version",
                    "schema_version": 7
                }
            }),
        ),
    ] {
        loop_state
            .capability_results
            .push(CapabilityResultEnvelope::ok(
                "db_basic",
                Some(action.to_string()),
                data,
            ));
    }

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn archive_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    for (action, data) in [
        (
            "list",
            json!({"extra": {"members": ["notes.txt"], "member_count": 1}}),
        ),
        (
            "read",
            json!({"extra": {"member_path": "notes.txt", "content_excerpt": "release notes"}}),
        ),
        ("pack", json!({"extra": {"archive": "/tmp/reports.zip"}})),
        ("unpack", json!({"extra": {"dest": "/tmp/reports"}})),
    ] {
        loop_state
            .capability_results
            .push(CapabilityResultEnvelope::ok(
                "archive_basic",
                Some(action.to_string()),
                data,
            ));
    }

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn git_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    for (action, data) in [
        (
            "status",
            json!({
                "extra": {
                    "action": "status",
                    "current_branch": "main",
                    "clean": false,
                    "changed_count": 2,
                    "paths": ["Cargo.toml", "src/main.rs"]
                }
            }),
        ),
        (
            "log",
            json!({
                "extra": {
                    "action": "log",
                    "subject": "refactor: simplify delivery",
                    "subjects": ["refactor: simplify delivery"]
                }
            }),
        ),
    ] {
        loop_state
            .capability_results
            .push(CapabilityResultEnvelope::ok(
                "git_basic",
                Some(action.to_string()),
                data,
            ));
    }

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn config_key_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "config_basic",
            Some("list_keys".to_string()),
            json!({
                "extra": {
                    "action": "structured_keys",
                    "exists": true,
                    "container_type": "object",
                    "count": 3,
                    "keys": ["model", "runtime", "skills"]
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn config_field_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "config_basic",
            Some("read_field".to_string()),
            json!({
                "extra": {
                    "action": "extract_field",
                    "field_path": "llm.selected_vendor",
                    "exists": true,
                    "value": "minimax",
                    "value_text": "minimax",
                    "value_type": "string"
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn config_risk_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "config_edit",
            Some("guard_config".to_string()),
            json!({
                "extra": {
                    "action": "guard_config",
                    "path": "configs/config.toml",
                    "valid": false,
                    "risk_count": 1,
                    "count": 1,
                    "candidates": ["tools.allow_sudo=true"]
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn multiple_structured_fields_use_generic_synthesis_without_comparison_contract() {
    let mut loop_state = LoopState::default();
    for (path, field_path, value) in [
        ("UI/package.json", "name", "rustclaw-ui"),
        ("crates/clawd/Cargo.toml", "package.name", "clawd"),
    ] {
        loop_state
            .capability_results
            .push(CapabilityResultEnvelope::ok(
                "config_basic",
                Some("read_field".to_string()),
                json!({
                    "extra": {
                        "action": "read_field",
                        "path": path,
                        "field_path": field_path,
                        "exists": true,
                        "value": value,
                        "value_text": value,
                        "value_type": "string"
                    }
                }),
            ));
    }

    assert_eq!(loop_state.capability_results.len(), 2);
    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn read_range_title_result_uses_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "system_basic",
            Some("read_range".to_string()),
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "docs/service_notes.md",
                    "field_selector": "title",
                    "title": "Service Notes",
                    "exists": true
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn read_range_excerpt_uses_generic_judgment_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "fs_basic",
            Some("read_text_range".to_string()),
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "docs/release_checklist.md",
                    "excerpt": "1|# Release Checklist\n2|Verify config loading.",
                    "start_line": 1,
                    "end_line": 2
                }
            }),
        ));
    let route = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "docs/release_checklist.md".to_string(),
        ..Default::default()
    };
    let context = AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&context)
    ));
}

#[test]
fn path_facts_result_uses_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "system_basic",
            Some("path_batch_facts".to_string()),
            json!({
                "extra": {
                    "action": "path_batch_facts",
                    "basename": "release_checklist.md",
                    "count": 1
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn compound_path_existence_and_content_use_generic_synthesis() {
    let mut loop_state = LoopState::default();
    loop_state.capability_results.extend([
        CapabilityResultEnvelope::ok(
            "system_basic",
            Some("path_batch_facts".to_string()),
            json!({
                "extra": {
                    "action": "path_batch_facts",
                    "facts": [{"path": "Cargo.toml", "exists": true, "kind": "file"}]
                }
            }),
        ),
        CapabilityResultEnvelope::ok(
            "system_basic",
            Some("read_range".to_string()),
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "Cargo.toml",
                    "excerpt": "1|[workspace]"
                }
            }),
        ),
    ]);
    let route = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "Cargo.toml".to_string(),
        ..Default::default()
    };
    let context = AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&context)
    ));
}

#[test]
fn workspace_inventory_and_read_excerpt_use_generic_synthesis() {
    let mut loop_state = LoopState::default();
    loop_state.capability_results.extend([
        CapabilityResultEnvelope::ok(
            "fs_basic",
            Some("list_dir".to_string()),
            json!({
                "extra": {
                    "action": "list_dir",
                    "path": ".",
                    "entries": [
                        {"name": "crates", "kind": "dir"},
                        {"name": "UI", "kind": "dir"},
                        {"name": "README.md", "kind": "file"}
                    ]
                }
            }),
        ),
        CapabilityResultEnvelope::ok(
            "fs_basic",
            Some("read_text_range".to_string()),
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "README.md",
                    "excerpt": "1|# RustClaw\n2|A local agent runtime."
                }
            }),
        ),
    ]);
    let route = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    let context = AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&context)
    ));
}

#[test]
fn mtime_ranked_listing_and_excerpts_use_generic_judgment_synthesis() {
    let mut loop_state = LoopState::default();
    loop_state.capability_results.extend([
        CapabilityResultEnvelope::ok(
            "fs_basic",
            Some("list_dir".to_string()),
            json!({
                "extra": {
                    "action": "list_dir",
                    "path": "docs",
                    "sort_by": "mtime_desc",
                    "entries": [
                        {
                            "name": "release.md",
                            "kind": "file",
                            "modified_ts": 200,
                            "path": "docs/release.md"
                        },
                        {
                            "name": "notes.md",
                            "kind": "file",
                            "modified_ts": 100,
                            "path": "docs/notes.md"
                        }
                    ]
                }
            }),
        ),
        CapabilityResultEnvelope::ok(
            "fs_basic",
            Some("read_text_range".to_string()),
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "docs/release.md",
                    "excerpt": "1|# Release Checklist"
                }
            }),
        ),
    ]);
    let route = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "docs".to_string(),
        selection: crate::OutputSelectionContract {
            list_selector: crate::pipeline_types::OutputListSelector {
                target_kind: crate::pipeline_types::OutputScalarCountTargetKind::File,
                target_kind_specified: true,
                limit: Some(2),
                sort_by: Some("mtime_desc".to_string()),
                include_metadata: Some(true),
                include_hidden: Some(false),
            },
            structured_field_selector: None,
        },
        ..Default::default()
    };
    let context = AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&context)
    ));
    let bounded = bounded_result(&loop_state.capability_results[0]);
    assert_eq!(
        bounded.data.pointer("/extra/sort_by"),
        Some(&json!("mtime_desc"))
    );
    assert_eq!(
        bounded.data.pointer("/extra/entries/0/modified_ts"),
        Some(&json!(200))
    );
}

#[test]
fn grep_results_use_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "fs_basic",
            Some("grep_text".to_string()),
            json!({
                "extra": {
                    "action": "grep_text",
                    "root": "docs",
                    "query": "release",
                    "match_count": 1,
                    "matches": [{
                        "path": "docs/release_checklist.md",
                        "line": 1,
                        "text": "# Release Checklist"
                    }]
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn exact_machine_and_artifact_delivery_bypass_language_synthesis() {
    let mut loop_state = LoopState::default();
    let mut result =
        CapabilityResultEnvelope::ok("filesystem.read", Some("read".to_string()), json!({}));
    result.delivery = CapabilityDelivery {
        intent: CapabilityDeliveryIntent::ExactMachine,
        constraints: json!({}),
    };
    loop_state.capability_results.push(result);
    assert!(!eligible_for_capability_result_synthesis(&loop_state, None));
}

#[test]
fn oversized_result_is_bounded_without_changing_machine_identity() {
    let result = CapabilityResultEnvelope::ok(
        "filesystem.read",
        Some("read".to_string()),
        json!({"content": "x".repeat(MAX_RESULT_JSON_CHARS + 10_000)}),
    );
    let bounded = bounded_result(&result);
    assert_eq!(bounded.capability, result.capability);
    assert_eq!(bounded.action, result.action);
    assert!(bounded.data.to_string().chars().count() < MAX_RESULT_JSON_CHARS);
}
