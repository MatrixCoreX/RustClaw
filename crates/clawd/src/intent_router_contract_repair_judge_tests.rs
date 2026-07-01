use super::{OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind};
use crate::IntentOutputContract;
use serde_json::json;
use std::collections::BTreeSet;

fn contract_repair_semantic_kind_enum() -> BTreeSet<String> {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../prompts/schemas/contract_repair_judge.schema.json"
    ))
    .expect("contract repair judge schema parses");
    schema
        .pointer("/properties/output_contract/properties/semantic_kind/enum")
        .and_then(|value| value.as_array())
        .expect("semantic_kind enum")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[test]
fn contract_repair_judge_schema_hides_registry_capability_semantic_kinds() {
    let schema_semantic_kinds = contract_repair_semantic_kind_enum();

    for kind in OutputSemanticKind::ALL {
        if kind.is_normalizer_schema_capability_bridge() {
            assert!(
                !schema_semantic_kinds.contains(kind.as_str()),
                "contract repair judge schema must not expose registry-owned semantic_kind `{}`; preserve capability_ref machine tokens instead",
                kind.as_str()
            );
        }
    }
}

#[test]
fn contract_repair_judge_schema_accepts_canonical_payload() {
    let raw = r#"{
          "apply": true,
          "reason": "malformed_contract_semantically_requires_directory_listing",
          "confidence": 0.91,
          "decision": "planner_execute",
          "needs_clarify": false,
          "clarify_question": "",
          "resolved_user_intent": "列出 logs 目录下前 3 个文件名，不读取内容",
          "output_contract": {
            "response_shape": "strict",
            "exact_sentence_count": null,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "file_names",
            "locator_hint": "logs",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
          },
          "execution_recipe": {"kind": "none", "profile": "none", "target_scope": "unknown"},
          "turn_type": "task_request",
          "target_task_policy": "standalone"
        }"#;

    crate::prompt_utils::validate_against_schema::<super::ContractRepairJudgeOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::ContractRepairJudge,
    )
    .expect("contract repair judge payload should validate");
}

#[test]
fn contract_repair_judge_rejects_scalar_semantic_repair_without_machine_marker() {
    let raw = r#"{
          "apply": true,
          "reason": "memory_only_answer_candidate_conflict_with_current_file_read_request",
          "confidence": 0.91,
          "decision": "planner_execute",
          "needs_clarify": false,
          "clarify_question": "",
          "resolved_user_intent": "read package.json name field",
          "output_contract": {
            "response_shape": "strict",
            "exact_sentence_count": null,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "scalar",
            "locator_hint": "scripts/nl_tests/fixtures/device_local/package.json",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
          },
          "execution_recipe": {
            "kind": "structured_read",
            "profile": "read_only",
            "target_scope": "explicit_path"
          },
          "turn_type": "task_request",
          "target_task_policy": "standalone"
        }"#;

    let validated = crate::prompt_utils::validate_against_schema::<super::ContractRepairJudgeOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::ContractRepairJudge,
    )
    .expect("contract repair judge payload should validate");

    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "read package.json name field".to_string(),
        answer_candidate: "rustclaw-nl-fixture".to_string(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "direct answer candidate lacked current evidence".to_string(),
        confidence: 0.5,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(!super::apply_contract_repair_judge_output(
        &mut out,
        validated.value
    ));

    assert_eq!(out.decision, "direct_answer");
    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn contract_repair_judge_rejects_directory_semantic_repair_without_machine_marker() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "列出 document 目录下的所有文件名".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "malformed recipe text was ignored".to_string(),
        confidence: 0.5,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "malformed_contract_semantically_requires_directory_listing".to_string(),
        repair_target: String::new(),
        confidence: 0.91,
        decision: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "列出 document 目录下所有文件名，只输出文件名列表".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "file_names".to_string(),
            locator_hint: "document".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: None,
    };

    assert!(!super::apply_contract_repair_judge_output(&mut out, repair));

    assert_eq!(out.decision, "direct_answer");
    assert_eq!(out.confidence, 0.5);
    assert!(!out.reason.contains("contract_repair_applied"));
    assert!(!out.reason.contains("contract_repair_note_present"));
    assert!(!out.reason.contains("contract_repair_target=file_names"));
    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(!contract.requires_content_evidence);
    assert!(contract.locator_hint.is_empty());
    assert!(out.state_patch.is_none());
}

#[test]
fn contract_repair_judge_machine_marker_restores_execution_failed_step_contract() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "Review the prior observed execution failure.".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: true,
        clarify_question: String::new(),
        reason: "active task binding used non-canonical tokens".to_string(),
        confidence: 0.5,
        decision: "clarify".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason:
            "semantic repair `execution_failed_step_contract_preserves_ordered_command_sequence`"
                .to_string(),
        repair_target: String::new(),
        confidence: 0.91,
        decision: "clarify".to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        resolved_user_intent: "Review the prior observed execution failure.".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: true,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "file_single".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "/tmp/not-used".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: None,
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    assert_eq!(out.decision, "planner_execute");
    assert!(!out.needs_clarify);
    assert!(out.clarify_question.is_empty());
    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExecutionFailedStep
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn contract_repair_judge_generated_file_delivery_runtime_target_overrides_clarify() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "写一个脚本，保存并发送给用户".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: true,
        clarify_question: "需要保存到哪个文件路径？".to_string(),
        reason: "generated artifact delivery".to_string(),
        confidence: 0.5,
        decision: "clarify".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason:
            "generated_file_delivery_allows_runtime_target; direct_file_delivery_workspace_root_locator_rejected"
                .to_string(),
        repair_target: String::new(),
        confidence: 0.91,
        decision: "clarify".to_string(),
        needs_clarify: true,
        clarify_question: "需要保存到哪个文件路径？".to_string(),
        resolved_user_intent: "写一个脚本，保存到工作区文件，并作为附件发送给用户".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "file_token".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: "none".to_string(),
            delivery_intent: "file_single".to_string(),
            semantic_kind: "generated_file_delivery".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: None,
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    assert_eq!(out.decision, "planner_execute");
    assert!(!out.needs_clarify);
    assert!(out.clarify_question.is_empty());
    assert!(out.wants_file_delivery);
    let contract = super::parse_output_contract(out.output_contract, out.wants_file_delivery);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.requires_content_evidence);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn generated_file_delivery_contract_clears_spurious_attachment_processing() {
    let mut attachment_processing_required = true;
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        ..IntentOutputContract::default()
    };

    let repair = super::clear_spurious_generated_file_delivery_attachment_processing(
        &mut attachment_processing_required,
        &contract,
        true,
    );

    assert_eq!(
        repair,
        Some("generated_file_delivery_cleared_spurious_attachment_processing")
    );
    assert!(!attachment_processing_required);
}

#[test]
fn delivery_contract_clears_spurious_attachment_processing_without_semantic_kind() {
    let mut attachment_processing_required = true;
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let repair = super::clear_spurious_generated_file_delivery_attachment_processing(
        &mut attachment_processing_required,
        &contract,
        true,
    );

    assert_eq!(
        repair,
        Some("generated_file_delivery_cleared_spurious_attachment_processing")
    );
    assert!(!attachment_processing_required);
}

#[test]
fn contract_repair_judge_machine_marker_reuses_active_completed_task_status() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "Continue from the prior task.".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: true,
        clarify_question: String::new(),
        reason: "active task binding used non-canonical tokens".to_string(),
        confidence: 0.5,
        decision: "clarify".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"deictic_reference": {"target": "stale"}})),
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "active_task_invalid_turn_binding_repaired_continuation_request".to_string(),
        repair_target: String::new(),
        confidence: 0.91,
        decision: "clarify".to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        resolved_user_intent: "Continue from the prior task.".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "file_single".to_string(),
            semantic_kind: "content_excerpt_summary".to_string(),
            locator_hint: "/tmp/not-used".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: Some(json!({"deictic_reference": {"target": "stale_repair"}})),
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    assert_eq!(out.decision, "direct_answer");
    assert!(!out.needs_clarify);
    assert!(out.clarify_question.is_empty());
    assert_eq!(out.turn_type, "status_query");
    assert_eq!(out.target_task_policy, "reuse_active");
    assert!(out
        .reason
        .contains("contract_repair_marker=active_task_invalid_turn_binding"));
    assert!(out.state_patch.is_none());
    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert!(!contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn contract_repair_judge_preserves_structured_config_key_contract() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "读取 configs/config.toml 的顶层键名".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer chose structured keys".to_string(),
        confidence: 0.9,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "structured_keys".to_string(),
            locator_hint: "configs/config.toml".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "fresh_file_observation_required".to_string(),
        repair_target: String::new(),
        confidence: 0.95,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "读取 configs/config.toml 的顶层键名列表".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "configs/config.toml".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: None,
    };

    assert!(!super::apply_contract_repair_judge_output(&mut out, repair));

    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::StructuredKeys);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
    assert!(!out
        .reason
        .contains("structured_config_key_contract_preserved"));
}

#[test]
fn contract_repair_judge_preserves_structured_scalar_field_contract() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "extract a scalar field value from a structured file".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer chose structured scalar field contract".to_string(),
        confidence: 0.95,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "scalar".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "single_path_generic_contract_needs_semantic_shape_review".to_string(),
        repair_target: String::new(),
        confidence: 0.95,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "extract a scalar field value from a structured file".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "scalar".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "content_excerpt_summary".to_string(),
            locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: None,
    };

    assert!(!super::apply_contract_repair_judge_output(&mut out, repair));

    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/package.json"
    );
    assert!(!out
        .reason
        .contains("structured_scalar_field_contract_preserved"));
}

#[test]
fn contract_repair_judge_missing_turn_binding_forces_missing_locator_clarify() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "read remembered log alias".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "memory alias selected path".to_string(),
        confidence: 0.5,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "deictic_reference": {"target": "current_turn_locator"}
        })),
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "execution_recipe_untrusted_text_ignored_and_turn_binding_missing_for_content_read"
            .to_string(),
        repair_target: String::new(),
        confidence: 0.95,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "read remembered log alias".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "/repo/logs/app.log".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "fs_basic".to_string(),
            profile: "read_only".to_string(),
            target_scope: "explicit_path".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: Some(serde_json::json!({
            "deictic_reference": {"target": "current_turn_locator"}
        })),
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    assert_eq!(out.decision, "clarify");
    assert!(out.needs_clarify);
    let contract = super::parse_output_contract(out.output_contract, false);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert_eq!(
        out.state_patch
            .as_ref()
            .and_then(|patch| patch.get("deictic_reference"))
            .and_then(|value| value.get("target"))
            .and_then(serde_json::Value::as_str),
        Some("missing_locator")
    );
}

#[test]
fn contract_repair_judge_output_clears_stale_file_delivery_flag() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "Write a short release note for RustClaw".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: "RustClaw".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "malformed output contract".to_string(),
        confidence: 0.5,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "file_token".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: true,
            locator_kind: "path".to_string(),
            delivery_intent: "file_single".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "inline_text_contract".to_string(),
        repair_target: String::new(),
        confidence: 0.85,
        decision: "direct_answer".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "Write a short release note for RustClaw as inline content."
            .to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "none".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: String::new(),
        target_task_policy: String::new(),
        state_patch: None,
    };

    assert!(!super::apply_contract_repair_judge_output(&mut out, repair));

    assert!(out.wants_file_delivery);
    let contract = super::parse_output_contract(out.output_contract, out.wants_file_delivery);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
}

#[test]
fn contract_repair_judge_output_rejects_low_confidence() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "总结刚才的对话".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "uncertain".to_string(),
        repair_target: String::new(),
        confidence: 0.59,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "bad".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "file_names".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        state_patch: None,
    };

    assert!(!super::apply_contract_repair_judge_output(&mut out, repair));
    assert_eq!(out.decision, "direct_answer");
    assert_eq!(out.resolved_user_intent, "总结刚才的对话");
}

#[test]
fn contract_repair_judge_rejects_decision_change_without_machine_contract_signal() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "summarize the prior discussion".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "ordinary_semantic_route_change".to_string(),
        repair_target: String::new(),
        confidence: 0.91,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "summarize the prior discussion".to_string(),
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        state_patch: None,
    };

    assert!(!super::apply_contract_repair_judge_output(&mut out, repair));
    assert_eq!(out.decision, "direct_answer");
    assert!(out.output_contract.is_some());
    assert!(out.execution_recipe.is_some());
}
