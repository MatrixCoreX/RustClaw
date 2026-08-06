use claw_core::capability_result::{
    CapabilityDelivery, CapabilityDeliveryIntent, CapabilityResultEnvelope, CapabilityResultStatus,
    Continuation, ContinuationKind, StructuredError,
};
use serde_json::json;

use super::{
    bounded_result, eligible_for_capability_result_synthesis, normalized_transcript_language,
    pending_transcript_review, safe_transcript_filename, split_transcript_chunks,
    synthesis_evidence_catalog, transcript_delivery_is_inline, transcript_review_contract,
    FALLBACK_TRANSCRIPT_REVISION_CHUNK_CHARS, MAX_RESULT_JSON_CHARS,
};
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
fn latest_successful_transcription_contract_drives_review() {
    let mut earlier_result = CapabilityResultEnvelope::ok(
        "media_download.download",
        Some("download".to_string()),
        json!({"extra": {"saved_files": [{"artifact_role": "original_video"}]}}),
    );
    earlier_result.delivery.intent = CapabilityDeliveryIntent::Silent;
    let results = vec![
        earlier_result,
        CapabilityResultEnvelope::ok(
            "audio.transcribe",
            Some("transcribe".to_string()),
            json!({
                "extra": {
                    "transcription_review": {
                        "required": true,
                        "raw_text": "first",
                        "response_language": "en",
                        "source": "configured_stt"
                    }
                }
            }),
        ),
        CapabilityResultEnvelope::ok(
            "media_download.transcribe",
            Some("transcribe".to_string()),
            json!({
                "extra": {
                    "transcription_review": {
                        "required": true,
                        "raw_text": "今天天汽很好",
                        "response_language": "zh-CN",
                        "source": "media_download_local_asr",
                        "delivery": {
                            "inline_max_characters_exclusive": 200,
                            "long_text_filename": "transcript.txt"
                        }
                    }
                }
            }),
        ),
    ];

    let contract = transcript_review_contract(&results).expect("review contract");
    assert!(pending_transcript_review(&results));
    assert_eq!(contract.result_index, 2);
    assert_eq!(contract.raw_text, "今天天汽很好");
    assert_eq!(contract.response_language, "zh-CN");
    assert_eq!(contract.inline_max_chars, 200);
    assert_eq!(contract.long_text_filename, "transcript.txt");

    let loop_state = LoopState {
        capability_results: results,
        ..LoopState::default()
    };
    let context = AgentRunContext {
        output_contract: Some(crate::IntentOutputContract {
            delivery_required: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&context)
    ));
    assert!(pending_transcript_review(&loop_state.capability_results));
}

#[test]
fn transcript_review_requires_its_own_terminal_model_synthesis_result() {
    let mut silent = CapabilityResultEnvelope::ok(
        "media_download.transcribe",
        Some("transcribe".to_string()),
        json!({
            "extra": {
                "transcription_review": {
                    "required": true,
                    "raw_text": "待校对文本"
                }
            }
        }),
    );
    silent.delivery.intent = CapabilityDeliveryIntent::Silent;
    assert!(!pending_transcript_review(&[silent]));

    let mut waiting = CapabilityResultEnvelope::ok(
        "media_download.transcribe",
        Some("transcribe".to_string()),
        json!({
            "extra": {
                "transcription_review": {
                    "required": true,
                    "raw_text": "待校对文本"
                }
            }
        }),
    );
    waiting.status = CapabilityResultStatus::Waiting;
    waiting.continuation = Some(Continuation {
        kind: ContinuationKind::Poll,
        reference: Some("job-transcript".to_string()),
        poll_after_ms: Some(1_000),
        state: json!({}),
    });
    assert!(!pending_transcript_review(&[waiting]));
}

#[test]
fn transcript_chunks_are_unicode_safe_and_bounded() {
    let source = format!(
        "{}。{}",
        "甲".repeat(FALLBACK_TRANSCRIPT_REVISION_CHUNK_CHARS),
        "乙".repeat(500)
    );
    let chunks = split_transcript_chunks(&source, FALLBACK_TRANSCRIPT_REVISION_CHUNK_CHARS);

    assert!(chunks.len() >= 2);
    assert!(chunks
        .iter()
        .all(|chunk| chunk.chars().count() <= FALLBACK_TRANSCRIPT_REVISION_CHUNK_CHARS));
    assert_eq!(chunks.concat(), source);
}

#[test]
fn transcript_language_and_filename_are_safely_normalized() {
    assert_eq!(
        normalized_transcript_language("request-language", "zh-CN"),
        "zh-CN"
    );
    assert_eq!(normalized_transcript_language("ja-JP", "zh-CN"), "ja-JP");
    let filename = safe_transcript_filename("../最终稿");
    assert!(!filename.contains('/'));
    assert!(filename.ends_with(".txt"));
}

#[test]
fn transcript_delivery_switches_to_file_at_two_hundred_characters() {
    assert!(transcript_delivery_is_inline(199, 200));
    assert!(!transcript_delivery_is_inline(200, 200));
    assert!(!transcript_delivery_is_inline(201, 200));
}

#[test]
fn transcript_revision_schema_accepts_only_complete_review_output() {
    let parsed = crate::prompt_utils::validate_against_schema::<super::TranscriptRevisionOutput>(
        r#"{"reviewed_text":"校对后的文本。","delivery_message":"校对后的完整文本已作为附件发送。","qualified":true,"confidence":0.9,"reason":"complete"}"#,
        crate::prompt_utils::PromptSchemaId::TranscriptRevision,
    )
    .expect("valid transcript revision output");

    assert_eq!(parsed.value.reviewed_text, "校对后的文本。");
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
fn filesystem_mutation_receipt_uses_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "filesystem.write_text",
            Some("write_text".to_string()),
            json!({
                "extra": {
                    "path": "tmp/job-a/note.txt",
                    "content_bytes": 5,
                    "changed": true
                }
            }),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn structured_failure_uses_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::failed(
            "system.run_command",
            Some("run".to_string()),
            StructuredError {
                code: "command_failed".to_string(),
                message_key: "system.command_failed".to_string(),
                retryable: false,
                details: json!({"step_id": "step_2", "exit_code": 7}),
            },
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn mixed_success_failure_and_later_success_use_generic_synthesis() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "system.run_command",
            Some("run".to_string()),
            json!({"step_id": "step_1", "stdout": "before"}),
        ));
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::failed(
            "system.run_command",
            Some("run".to_string()),
            StructuredError {
                code: "command_failed".to_string(),
                message_key: "system.command_failed".to_string(),
                retryable: false,
                details: json!({"step_id": "step_2", "exit_code": 127}),
            },
        ));
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "system.run_command",
            Some("run".to_string()),
            json!({"step_id": "step_3", "stdout": "after"}),
        ));

    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn continuation_status_does_not_enter_terminal_synthesis() {
    let mut result =
        CapabilityResultEnvelope::ok("system.run_command", Some("run".to_string()), json!({}));
    result.status = CapabilityResultStatus::Waiting;
    result.continuation = Some(Continuation {
        kind: ContinuationKind::Poll,
        reference: Some("job-1".to_string()),
        poll_after_ms: Some(1_000),
        state: json!({}),
    });
    let mut loop_state = LoopState::default();
    loop_state.capability_results.push(result);

    assert!(!eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn config_validation_receipt_uses_generic_synthesis_without_domain_contract() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "config_basic",
            Some("validate".to_string()),
            json!({
                "extra": {
                    "path": "configs/config.toml",
                    "format": "toml",
                    "valid": true,
                    "root_type": "object"
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
        ("UI/package.json", "name", "agent-runtime-ui"),
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
                    "excerpt": "1|# Agent Runtime\n2|A local agent runtime."
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

#[test]
fn explicit_model_observation_keeps_deep_evidence_and_drops_bulk_metadata() {
    let result = CapabilityResultEnvelope::ok(
        "registry.fixture",
        Some("inspect".to_string()),
        json!({
            "extra": {
                "package": {"metadata": "x".repeat(20_000)},
                "model_observation": {
                    "workbook": {
                        "sheets": [{
                            "cells": [{
                                "reference": "B4",
                                "formula": "SUM(B2:B3)"
                            }]
                        }]
                    }
                }
            }
        }),
    );

    let bounded = bounded_result(&result);
    assert_eq!(
        bounded
            .data
            .pointer("/model_observation/workbook/sheets/0/cells/0/formula"),
        Some(&json!("SUM(B2:B3)"))
    );
    assert!(bounded.data.pointer("/extra/package").is_none());
}

#[test]
fn result_nine_and_large_result_remain_content_addressed_and_recoverable() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-evidence-catalog-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    state.skill_rt.workspace_root = root.clone();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut results = (0..9)
        .map(|index| {
            CapabilityResultEnvelope::ok(
                format!("fixture.capability_{index}"),
                Some("observe".to_string()),
                json!({"index": index, "complete": true}),
            )
        })
        .collect::<Vec<_>>();
    results.push(CapabilityResultEnvelope::ok(
        "fixture.large",
        Some("observe".to_string()),
        json!({"content": "x".repeat(256_000), "complete": true}),
    ));

    let catalog = synthesis_evidence_catalog(&state, &task, &results).unwrap();
    let entries = catalog["entries"].as_array().unwrap();
    assert_eq!(catalog["result_count"], 10);
    assert_eq!(entries.len(), 10);
    assert_eq!(
        entries[8]["evidence_id"],
        results[8].canonical_evidence_identity().evidence_id
    );
    let large = &entries[9];
    assert_eq!(large["model_view"]["complete"], false);
    assert_eq!(
        large["model_view"]["continuation"]["kind"],
        "artifact_range"
    );
    let relative = large["model_view"]["continuation"]["range_handle"]["path"]
        .as_str()
        .unwrap();
    let artifact = root.join(relative);
    assert!(artifact.is_file());
    assert_eq!(
        large["sha256"],
        results[9].canonical_evidence_identity().sha256
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            artifact.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
