// Normalizer schedule and turn-policy tests for intent_router.

use crate::runtime::types::{ScheduleIntentSchedule, ScheduleIntentTask};

use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, TargetTaskPolicy, TurnType,
};

#[test]
fn normalizer_schema_normalization_coerces_invalid_schedule_kind_to_none() {
    let raw = r#"{
          "resolved_user_intent":"修改方案目标用户为开发者，输出正文",
          "resume_behavior":"resume",
          "schedule_kind":"immediate",
          "schedule_intent":"deliver",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户修正目标受众约束并明确要求输出正文",
          "confidence":1.0,
          "decision":"direct_answer",
          "output_contract":{"kind":"text","text_content":"示例正文","media_type":"text/plain"},
          "execution_recipe":{"kind":"none"},
          "turn_type":"task_append",
          "target_task_policy":"reuse_active",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "不对，目标用户改成开发者，不是老板。只输出修正后的正文。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("schedule_kind").and_then(|value| value.as_str()),
        Some("none")
    );
    assert!(value
        .get("schedule_intent")
        .is_some_and(|value| value.is_null()));
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("task_append")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("reuse_active")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_promotes_schedule_type_token_without_decision_authority() {
    let raw = r#"{
          "resolved_user_intent":"Create a daily 08:00 reminder in the current conversation",
          "resume_behavior":"none",
          "schedule_kind":"daily",
          "schedule_intent":"reminder",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"schedule type token was emitted in the operation field",
          "confidence":0.91,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"one_sentence","requires_content_evidence":false,"delivery_required":false,"locator_kind":"none","delivery_intent":"none","semantic_kind":"none","locator_hint":"","self_extension":{"mode":"none","trigger":"none","execute_now":false}},
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"none"},
          "turn_type":"task_request",
          "target_task_policy":"standalone",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "Create a daily 08:00 reminder in the current conversation",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("schedule_kind").and_then(|value| value.as_str()),
        Some("create")
    );
    assert!(value
        .get("schedule_intent")
        .is_some_and(|value| value.is_null()));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schedule_intent_missing_object_uses_schedule_compiler_later() {
    let intent = super::normalize_schedule_intent_from_normalizer(
        super::ScheduleKind::Create,
        None,
        "Create a daily 08:00 reminder in the current conversation",
        "schedule operation recognized",
        false,
        "",
        0.9,
    );
    assert!(intent.is_none());
}

#[test]
fn normalizer_schedule_intent_relative_trigger_at_uses_schedule_compiler_later() {
    let intent = crate::ScheduleIntentOutput {
        kind: "create".to_string(),
        mode: String::new(),
        dry_run: true,
        schedule: ScheduleIntentSchedule {
            r#type: "once".to_string(),
            run_at: "tomorrow 09:00:00 +08:00".to_string(),
            content: "check service".to_string(),
            ..Default::default()
        },
        task: ScheduleIntentTask {
            kind: String::new(),
            payload: serde_json::json!({}),
        },
        confidence: 0.9,
        ..Default::default()
    };
    let normalized = super::normalize_schedule_intent_from_normalizer(
        super::ScheduleKind::Create,
        Some(intent),
        "Create a one-time reminder tomorrow",
        "schedule operation recognized",
        false,
        "",
        0.9,
    );
    assert!(normalized.is_none());
}

#[test]
fn schedule_route_contract_repair_clears_filesystem_evidence_contract() {
    let mut contract = super::IntentOutputContract::default();
    contract.response_shape = super::OutputResponseShape::OneSentence;
    contract.requires_content_evidence = true;
    contract.delivery_required = false;
    contract.locator_kind = super::OutputLocatorKind::None;
    contract.delivery_intent = super::OutputDeliveryIntent::None;
    contract.semantic_kind = super::OutputSemanticKind::FilesystemMutationResult;
    let mut wants_file_delivery = false;
    let decision = super::FirstLayerDecision::DirectAnswer;
    let mut finalize_style = super::ActFinalizeStyle::ChatWrapped;

    let repair = super::apply_schedule_route_contract_repair(
        super::ScheduleKind::Create,
        &mut contract,
        &mut wants_file_delivery,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("schedule_route_contract_repair"));
    assert!(!wants_file_delivery);
    assert!(!contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, super::OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, super::OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, super::OutputSemanticKind::None);
    assert_eq!(decision, super::FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, super::ActFinalizeStyle::Plain);
}

#[test]
fn normalizer_schema_normalization_recovers_minimax_text_only_append_payload() {
    let raw = r#"{
          "resolved_user_intent":"调整字数约束：RustClaw 连续会话可靠性技术博客风格，100字以内",
          "answer_candidate":"RustClaw 以多层容错与自动状态恢复机制，在网络抖动或进程异常时快速回到上一状态，保障关键业务链路不中断。",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"none",
          "needs_clarify":false,
          "clarify_question":"none",
          "reason":"字数约束更新，主题、受众、语气不变，直接输出精简版本",
          "confidence":"0.97",
          "decision":"direct_answer",
          "output_contract":"text_only",
          "execution_recipe":{"kind":"none","requires_content_evidence":false,"locator_kind":"none"},
          "turn_type":"task_append",
          "target_task_policy":"reuse_active",
          "should_interrupt_active_run":"no",
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "不对，不是 200 字，是 100 字以内");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("task_append")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("reuse_active")
    );
    assert_eq!(
        value
            .get("should_interrupt_active_run")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("free")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_treats_one_line_comparison_as_strict_shape() {
    let raw = r#"{
          "resolved_user_intent":"比较两个字段并输出一行",
          "resume_behavior":null,
          "schedule_kind":"immediate",
          "schedule_intent":"read_and_compare",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"clear comparison",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"one_line_comparison",
          "execution_recipe":{"kind":"file_read_two"},
          "turn_type":"task_request",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "读取两个字段，最后只用一行输出：前者、后者、一样或不一样",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("strict")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|value| value.as_str()),
        Some("recent_scalar_equality_check")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_rejects_invalid_resume_and_turn_tokens() {
    let raw = r#"{
          "resolved_user_intent":"用户希望在80字以内生成一份面向开发者的简短方案，缺少主题信息。",
          "resume_behavior":"unsupported_resume_token",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"assistant",
          "needs_clarify":true,
          "clarify_question":"请告诉我这份方案的主题是什么？",
          "reason":"缺少核心主题",
          "confidence":0.95,
          "decision":"direct_answer",
          "output_contract":"clarification",
          "execution_recipe":{"kind":"none"},
          "turn_type":"unsupported_turn_token",
          "target_task_policy":"reuse active",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "不对，目标用户改成开发者，不是老板。只输出修正后的正文。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .get("resume_behavior")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("reuse_active")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_coerces_empty_state_patch_string() {
    let raw = r#"{
          "resolved_user_intent":"用户询问刚才记住的测试编号",
          "resume_behavior":"",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"上下文中已有编号",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":"",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":"",
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "刚才让你记住的连续测试编号是什么？只回答编号。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert!(value
        .get("state_patch")
        .is_some_and(|value| value.is_null()));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_promotes_misnested_turn_analysis_fields() {
    let raw = r#"{
          "resolved_user_intent":"用一句中文确认当前测试正在进行",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"pure confirmation",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"one_sentence"},
          "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"none",
            "turn_type":"task_request",
            "target_task_policy":"standalone",
            "state_patch":{"constraints":{"tone":"brief"}},
            "attachment_processing_required":true
          },
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "请用一句中文回复确认。");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("task_request")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("standalone")
    );
    assert_eq!(
        value
            .pointer("/state_patch/constraints/tone")
            .and_then(|value| value.as_str()),
        Some("brief")
    );
    assert_eq!(
        value
            .get("attachment_processing_required")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert!(value
        .get("execution_recipe")
        .and_then(|value| value.as_object())
        .is_some_and(|recipe| !recipe.contains_key("turn_type")
            && !recipe.contains_key("target_task_policy")
            && !recipe.contains_key("state_patch")));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_recovers_commands_payload_as_execution_signal() {
    let raw = r#"{
          "resolved_user_intent":"User wants to know approximate size of the target directory",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"User requested local directory size and provided a command payload.",
          "confidence":0.95,
          "decision":"direct_answer",
          "output_contract":"json",
          "execution_recipe":{"commands":[{"executor":"local","command":"du -sh target","purpose":"Get approximate size"}]},
          "turn_type":"task_request",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "看一下 target 大概多大");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/execution_recipe/kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_ignores_response_recipe_as_execution_signal() {
    let raw = r#"{
          "resolved_user_intent":"确认用户正在进行 RustClaw 真实客户端连续会话测试",
          "answer_candidate":"好的，我已确认你正在进行 RustClaw 的真实客户端连续会话测试。",
          "resume_behavior":null,
          "schedule_kind":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户明确要求用一句中文回复确认其正在进行 RustClaw 真实客户端连续会话测试",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"text","semantic_kind":"confirmation"},
          "execution_recipe":"respond_with_simple_chinese_confirmation",
          "turn_type":"greeting",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "你好，我正在做 RustClaw 的真实客户端连续会话测试，请用一句中文回复确认。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/execution_recipe/kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn structured_preference_ack_is_detached_from_active_task() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    assert!(super::should_detach_bare_acknowledgement_from_active_task(
        Some(TurnType::PreferenceOrMemory),
        Some(TargetTaskPolicy::ReuseActive),
        &contract,
        None,
        false,
    ));
    assert!(!super::should_detach_bare_acknowledgement_from_active_task(
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        &contract,
        None,
        false,
    ));
    assert!(!super::should_detach_bare_acknowledgement_from_active_task(
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        &contract,
        Some(&serde_json::json!({"output_refinement":"one_sentence_only"})),
        false,
    ));
}

#[test]
fn orphan_output_shape_clarify_downgrades_to_standalone_chat() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let snapshot_without_primary = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState::default()),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(
        super::orphan_output_shape_loop_context_hint(
            Some(&snapshot_without_primary),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            true,
            &contract,
            None,
            false,
            false,
        ),
        Some("orphan_output_shape_loop_context")
    );

    let snapshot_with_primary = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我写个方案".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(
        super::orphan_output_shape_loop_context_hint(
            Some(&snapshot_with_primary),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            true,
            &contract,
            None,
            false,
            false,
        ),
        None
    );
}

#[test]
fn missing_turn_type_with_standalone_policy_infers_primary_task_request() {
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::Standalone),
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskRequest)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            Some(TurnType::PreferenceOrMemory),
            Some(TargetTaskPolicy::Standalone),
            false,
            crate::ScheduleKind::None,
            true,
        ),
        Some(TurnType::PreferenceOrMemory)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::Standalone),
            true,
            crate::ScheduleKind::None,
            false,
        ),
        None
    );
}

#[test]
fn missing_turn_type_with_active_task_policy_infers_mutation_type() {
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::ReuseActive),
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskAppend)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::ReplaceActive),
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskReplace)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::ReuseActive),
            false,
            crate::ScheduleKind::None,
            true,
        ),
        None
    );
}

#[test]
fn standalone_freeform_clarify_emits_loop_context_hint() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    assert_eq!(
        super::standalone_freeform_clarify_loop_context_hint(
            None,
            Some(TurnType::TaskRequest),
            None,
            true,
            &contract,
            None,
            false,
            false,
            false,
            crate::ScheduleKind::None,
        ),
        Some("standalone_freeform_clarify_loop_context")
    );
    assert_eq!(
        super::standalone_freeform_clarify_loop_context_hint(
            None,
            Some(TurnType::TaskRequest),
            Some(TargetTaskPolicy::ReuseActive),
            true,
            &contract,
            None,
            false,
            false,
            false,
            crate::ScheduleKind::None,
        ),
        Some("standalone_freeform_clarify_loop_context")
    );
}

#[test]
fn standalone_freeform_clarify_loop_context_preserves_observable_and_active_tasks() {
    let observable_contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::WorkspaceProjectSummary,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    assert_eq!(
        super::standalone_freeform_clarify_loop_context_hint(
            None,
            Some(TurnType::TaskRequest),
            None,
            true,
            &observable_contract,
            None,
            false,
            false,
            false,
            crate::ScheduleKind::None,
        ),
        None
    );

    let snapshot_with_primary = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a draft".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let freeform_contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    assert_eq!(
        super::standalone_freeform_clarify_loop_context_hint(
            Some(&snapshot_with_primary),
            Some(TurnType::TaskRequest),
            None,
            true,
            &freeform_contract,
            None,
            false,
            false,
            false,
            crate::ScheduleKind::None,
        ),
        None
    );
}

#[test]
fn missing_policy_with_strict_chat_deliverable_infers_standalone_task() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let policy = super::infer_missing_target_policy_from_contract(
        None,
        None,
        false,
        crate::ScheduleKind::None,
        false,
        &contract,
    );
    assert_eq!(policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            policy,
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskRequest)
    );
}

#[test]
fn missing_policy_with_non_strict_chat_does_not_promote_generic_chat() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    assert_eq!(
        super::infer_missing_target_policy_from_contract(
            None,
            None,
            false,
            crate::ScheduleKind::None,
            false,
            &contract,
        ),
        None
    );
}

#[test]
fn empty_nested_state_patch_is_not_meaningful() {
    assert!(!super::is_meaningful_state_patch(&serde_json::json!({
        "alias_bindings": [],
        "notes": ""
    })));
    assert!(super::is_meaningful_state_patch(&serde_json::json!({
        "audience": "developers"
    })));
}

#[test]
fn normalizer_schema_documents_recent_count_quantity_state_patch() {
    const SCHEMA_RAW: &str = include_str!("../../../prompts/schemas/intent_normalizer.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("intent_normalizer.schema.json must parse");
    let description = schema
        .pointer("/properties/state_patch/description")
        .and_then(serde_json::Value::as_str)
        .expect("state_patch description should be present");

    assert!(description.contains("quantity_comparison"));
    assert!(description.contains("\"selection\":\"max\"|\"min\""));
    assert!(description.contains("\"source\":\"recent_count_inventory\""));
}
