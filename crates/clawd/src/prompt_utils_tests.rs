use serde_json::{json, Value};

#[test]
fn validate_against_schema_rejects_unknown_intent_decision() {
    let raw = r#"{
        "resolved_user_intent":"check logs",
        "decision":"oops_status",
        "needs_clarify":false,
        "reason":"r",
        "confidence":0.9
    }"#;
    let err = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
        .expect_err("unknown decision should fail schema validation");
    assert!(err.to_string().contains("$.decision"));
    assert!(err.to_string().contains("oops_status"));
}

#[test]
fn validate_against_schema_rejects_out_of_range_finalizer_confidence() {
    let raw = r#"{
        "answer":"done",
        "qualified":true,
        "needs_clarify":false,
        "is_meta_instruction":false,
        "publishable":true,
        "confidence":1.5
    }"#;
    let err = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::FinalizerOut)
        .expect_err("confidence > 1 should fail schema validation");
    assert!(err.to_string().contains("$.confidence"));
}

#[test]
fn validate_against_schema_canonicalizes_bare_plan_array() {
    let raw = r#"[{"type":"respond","content":"done"}]"#;
    let validated = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
        .expect("bare array should canonicalize to steps envelope");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/type")
            .and_then(|v| v.as_str()),
        Some("respond")
    );
}

#[test]
fn validate_against_schema_canonicalizes_plan_action_alias_array() {
    let raw = r#"{"actions":[{"type":"respond","content":"done"}]}"#;
    let validated = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
        .expect("actions alias should canonicalize to steps envelope");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/content")
            .and_then(|v| v.as_str()),
        Some("done")
    );
    assert!(validated.value.get("actions").is_none());
}

#[test]
fn validate_against_schema_strips_plan_result_noise_fields() {
    let raw = r#"{
        "goal":"ignored envelope noise",
        "planner_notes":"kept",
        "steps":[
            {
                "type":"RESPOND",
                "content":"done",
                "id":"step_1",
                "description":"ignored action noise"
            }
        ]
    }"#;
    let validated = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
        .expect("plan_result noise fields should be stripped before schema validation");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/type")
            .and_then(|v| v.as_str()),
        Some("respond")
    );
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/content")
            .and_then(|v| v.as_str()),
        Some("done")
    );
    assert_eq!(
        validated
            .value
            .get("planner_notes")
            .and_then(|v| v.as_str()),
        Some("kept")
    );
    assert!(validated.value.get("goal").is_none());
    assert!(validated.value.pointer("/steps/0/id").is_none());
    assert!(validated.value.pointer("/steps/0/description").is_none());
}

#[test]
fn validate_against_schema_preserves_structured_respond_intent_fields() {
    let raw = r#"{
        "steps":[
            {
                "type":"RESPOND",
                "content":"",
                "terminal_intent":"clarify",
                "clarify_reason_code":"missing_locator",
                "missing_slot":"locator",
                "message_key":"clawd.msg.clarify.missing_locator",
                "field_path":"package.name",
                "locator_kind":"path",
                "description":"ignored action noise"
            }
        ]
    }"#;
    let validated = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
        .expect("structured respond intent should pass plan schema");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/terminal_intent")
            .and_then(Value::as_str),
        Some("clarify")
    );
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/missing_slot")
            .and_then(Value::as_str),
        Some("locator")
    );
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/message_key")
            .and_then(Value::as_str),
        Some("clawd.msg.clarify.missing_locator")
    );
    assert!(validated.value.pointer("/steps/0/description").is_none());
}

#[test]
fn validate_against_schema_normalizes_contract_repair_confidence_label() {
    let raw = r#"{
        "apply": true,
        "reason": "config_risk_assessment_contract_repair",
        "confidence": "high",
        "decision": "planner_execute",
        "needs_clarify": false,
        "clarify_question": "",
        "resolved_user_intent": "Check configs/config.toml for obvious configuration issues.",
        "output_contract": {
            "response_shape": "one_sentence",
            "exact_sentence_count": 1,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "config_risk_assessment",
            "locator_hint": "/home/guagua/rustclaw/configs/config.toml",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
        },
        "execution_recipe": {"kind": "none", "profile": "none", "target_scope": "none"},
        "turn_type": "task_request",
        "target_task_policy": "standalone",
        "state_patch": null
    }"#;

    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::ContractRepairJudge)
            .expect("confidence labels from repair judge should be canonicalized");

    assert!(validated.schema_normalized);
    assert_eq!(
        validated.value.get("confidence").and_then(|v| v.as_f64()),
        Some(0.9)
    );
    assert_eq!(
        validated
            .value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("config_risk_assessment")
    );
}

#[test]
fn validate_against_schema_canonicalizes_single_plan_step_object() {
    let raw = r#"{"steps":{"type":"respond","content":"done"}}"#;
    let validated = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
        .expect("single object steps should canonicalize to steps array");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/steps/0/type")
            .and_then(|v| v.as_str()),
        Some("respond")
    );
}

#[test]
fn fenced_plan_parser_keeps_inner_markdown_fence_in_respond_content() {
    let raw = "模型说明。\n\n```json\n{\"steps\":[{\"type\":\"respond\",\"content\":\"前 15 行：\\n```\\n#!/usr/bin/env bash\\nset -euo pipefail\\n```\\n\\n这是一个重启 clawd 服务的脚本。\"}]}\n```\n";
    let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
        .expect("fenced plan with nested markdown fence should parse");
    let content = parsed
        .pointer("/steps/0/content")
        .and_then(|v| v.as_str())
        .expect("respond content should be preserved");
    assert!(content.contains("#!/usr/bin/env bash"));
    assert!(content.contains("这是一个重启 clawd 服务的脚本"));
}

#[test]
fn validate_against_schema_drops_execution_recipe_stray_fields() {
    let raw = r#"{
        "resolved_user_intent":"列出 document 目录下前 5 个文件名",
        "decision":"planner_execute",
        "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"filename",
            "delivery_intent":"none",
            "semantic_kind":"none"
        },
        "needs_clarify":false,
        "reason":"r",
        "confidence":0.92,
        "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"current_repo",
            "unknown_extra_text":"wrong place",
            "unknown_extra_score":0.61
        }
    }"#;
    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
            .expect("model-noise execution_recipe stray fields should be dropped");
    assert!(validated.schema_normalized);
    assert!(validated
        .value
        .get("execution_recipe")
        .and_then(|v| v.get("unknown_extra_text"))
        .is_none());
}

#[test]
fn validate_against_schema_normalizes_contract_repair_judge_payload_noise() {
    let raw = r#"{
        "apply": true,
        "reason": "semantic repair",
        "repair_target": "directory_purpose_summary",
        "confidence": 0.92,
        "decision":"planner_execute",
        "needs_clarify": false,
        "clarify_question": "",
        "resolved_user_intent": "find README candidates",
        "agent_display_name_hint": "extra field from model",
        "output_contract": {
            "response_shape": "list",
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "file",
            "delivery_intent": "none",
            "semantic_kind": "path_list",
            "locator_hint": "README",
            "unused": "drop me",
            "self_extension": {
                "mode": "none",
                "trigger": "none",
                "execute_now": false
            }
        },
        "execution_recipe": {
            "kind": "none",
            "profile": "none",
            "target_scope": "none",
            "unexpected": "drop me"
        }
    }"#;

    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::ContractRepairJudge)
            .expect("contract repair judge output should tolerate harmless model noise");

    assert!(validated.schema_normalized);
    assert!(validated.value.get("agent_display_name_hint").is_none());
    assert_eq!(
        validated.value.get("repair_target").and_then(Value::as_str),
        Some("directory_purpose_summary")
    );
    assert_eq!(
        validated
            .value
            .pointer("/output_contract/response_shape")
            .and_then(Value::as_str),
        Some("strict")
    );
    assert_eq!(
        validated
            .value
            .pointer("/output_contract/semantic_kind")
            .and_then(Value::as_str),
        Some("file_paths")
    );
    assert_eq!(
        validated
            .value
            .pointer("/execution_recipe/target_scope")
            .and_then(Value::as_str),
        Some("unknown")
    );
    assert!(validated
        .value
        .pointer("/execution_recipe/unexpected")
        .is_none());
}

#[test]
fn validate_against_schema_infers_missing_contract_repair_apply_for_executable_repair() {
    let raw = r#"{
        "resolved_user_intent": "List top-level TOML files and summarize their purpose.",
        "reason": "file_names is too strict for a purpose summary",
        "confidence": 0.92,
        "decision": "planner_execute",
        "needs_clarify": false,
        "clarify_question": "",
        "output_contract": {
            "response_shape": "one_sentence",
            "exact_sentence_count": null,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "current_workspace",
            "delivery_intent": "none",
            "semantic_kind": "directory_purpose_summary",
            "locator_hint": "",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
        },
        "execution_recipe": {"kind": "none", "profile": "none", "target_scope": "none"},
        "turn_type": "task_request",
        "target_task_policy": "standalone"
    }"#;

    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::ContractRepairJudge)
            .expect("missing apply should be inferred for complete executable repair");

    assert!(validated.schema_normalized);
    assert_eq!(
        validated.value.get("apply").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        validated
            .value
            .pointer("/output_contract/semantic_kind")
            .and_then(Value::as_str),
        Some("directory_purpose_summary")
    );
}

#[test]
fn validate_against_schema_infers_missing_contract_repair_apply_false_for_conservative_chat() {
    let raw = r#"{
        "resolved_user_intent": "Acknowledge the user.",
        "reason": "pure chat",
        "confidence": 0.92,
        "decision": "direct_answer",
        "needs_clarify": false,
        "clarify_question": "",
        "output_contract": {
            "response_shape": "free",
            "requires_content_evidence": false,
            "delivery_required": false,
            "locator_kind": "none",
            "delivery_intent": "none",
            "semantic_kind": "none",
            "locator_hint": "",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
        },
        "execution_recipe": {"kind": "none", "profile": "none", "target_scope": "none"},
        "turn_type": "",
        "target_task_policy": ""
    }"#;

    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::ContractRepairJudge)
            .expect("missing apply should stay false for conservative no-execution repair");

    assert!(validated.schema_normalized);
    assert_eq!(
        validated.value.get("apply").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn validate_against_schema_repairs_execution_recipe_locator_hint() {
    let raw = r#"{
        "resolved_user_intent":"列出 document 目录下前 5 个文件名",
        "decision":"planner_execute",
        "needs_clarify":false,
        "reason":"r",
        "confidence":0.95,
        "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":false,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"none"
        },
        "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"current_repo",
            "locator_hint":"document"
        }
    }"#;
    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
            .expect("execution_recipe.locator_hint should be canonicalized");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/output_contract/locator_hint")
            .and_then(|v| v.as_str()),
        Some("document")
    );
    assert!(validated
        .value
        .pointer("/execution_recipe/locator_hint")
        .is_none());
}

#[test]
fn validate_against_schema_repairs_execution_recipe_self_extension() {
    let raw = r#"{
        "resolved_user_intent":"检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
        "decision":"planner_execute",
        "needs_clarify":false,
        "reason":"r",
        "confidence":0.95,
        "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":false,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"existence_with_path",
            "locator_hint":"rustclaw.service"
        },
        "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"current_repo",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
        }
    }"#;
    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
            .expect("execution_recipe.self_extension should be canonicalized");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/output_contract/self_extension/mode")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert!(validated
        .value
        .pointer("/execution_recipe/self_extension")
        .is_none());
}

#[test]
fn validate_against_schema_repairs_execution_recipe_reason() {
    let raw = r#"{
        "resolved_user_intent":"列出 logs 目录下前 5 个文件名（按顺序编号）",
        "decision":"planner_execute",
        "needs_clarify":false,
        "reason":"r",
        "confidence":0.92,
        "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":"logs"
        },
        "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"current_repo",
            "reason":"scope is clear"
        }
    }"#;
    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
            .expect("execution_recipe.reason should be canonicalized away");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/execution_recipe/kind")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert!(validated
        .value
        .pointer("/execution_recipe/reason")
        .is_none());
}

#[test]
fn validate_against_schema_repairs_execution_recipe_qualifier() {
    let raw = r#"{
        "resolved_user_intent":"执行基础健康检查，只列最重要的结论",
        "decision":"planner_execute",
        "needs_clarify":false,
        "reason":"r",
        "confidence":0.92,
        "output_contract":{
            "response_shape":"one_sentence",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"service_status"
        },
        "execution_recipe":{
            "kind":"none",
            "profile":"ops_service",
            "target_scope":"system",
            "qualifier":""
        }
    }"#;
    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
            .expect("execution_recipe.qualifier should be dropped");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/execution_recipe/profile")
            .and_then(|v| v.as_str()),
        Some("ops_service")
    );
    assert!(validated
        .value
        .pointer("/execution_recipe/qualifier")
        .is_none());
}

#[test]
fn validate_against_schema_repairs_malformed_execution_recipe_boundary_field() {
    let raw = r#"{
        "resolved_user_intent":"查看当前 git 分支名称，只输出分支名",
        "decision":"planner_execute",
        "needs_clarify":false,
        "reason":"r",
        "confidence":0.95,
        "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":false,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"scalar_path_only"
        },
        "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"current_repo",
            "},\"unknown_extra_text":""
        },
        "unknown_extra_score":0.0
    }"#;
    let validated =
        super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
            .expect("malformed execution_recipe boundary field should be dropped");
    assert!(validated.schema_normalized);
    assert_eq!(
        validated
            .value
            .pointer("/execution_recipe/target_scope")
            .and_then(|v| v.as_str()),
        Some("current_repo")
    );
    assert!(validated
        .value
        .pointer("/execution_recipe/},\\\"unknown_extra_text")
        .is_none());
}

#[test]
fn parse_llm_json_raw_or_any_with_repair_handles_unescaped_quotes() {
    let raw = r#"{"resolved_user_intent":"记住："那玩意README"指向 /home/guagua/test/README.md","reason":"用户定义了"那玩意README"映射","confidence":1.0}"#;
    let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
        .expect("should parse repaired json");
    assert_eq!(
        parsed
            .get("resolved_user_intent")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
        "记住：\"那玩意README\"指向 /home/guagua/test/README.md"
    );
}

#[test]
fn parse_llm_json_raw_or_any_with_repair_dedupes_object_keys_for_struct() {
    use serde::Deserialize;
    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct ExecutionRecipeProbe {
        kind: String,
        target_scope: String,
    }
    let raw = r#"{"kind":"none","target_scope":"system","target_scope":"system"}"#;
    // Sanity check: 直接 derive Deserialize 在 duplicate field 上会失败。
    assert!(serde_json::from_str::<ExecutionRecipeProbe>(raw).is_err());
    let parsed = super::parse_llm_json_raw_or_any_with_repair::<ExecutionRecipeProbe>(raw)
        .expect("dedup pass should recover duplicate-key object");
    assert_eq!(
        parsed,
        ExecutionRecipeProbe {
            kind: "none".to_string(),
            target_scope: "system".to_string(),
        }
    );
}

#[test]
fn parse_llm_json_raw_or_any_with_repair_dedupes_nested_duplicate_keys() {
    let raw = r#"{"decision":"planner_execute","execution_recipe":{"kind":"none","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#;
    let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
        .expect("nested duplicate keys should be repaired");
    assert_eq!(
        parsed
            .pointer("/execution_recipe/target_scope")
            .and_then(|v| v.as_str()),
        Some("system")
    );
    assert_eq!(
        parsed.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
}

/// §F3-a：补齐缺失尾括号 + 测试 adv12 真实 MiniMax 输出。
#[test]
fn balance_unclosed_brackets_recovers_truncated_object() {
    // 完整对象本身已平衡，应返回 None（不重复劳动）。
    assert!(super::balance_unclosed_brackets(r#"{"a":1}"#).is_none());
    // 简单缺一个 `}`。
    assert_eq!(
        super::balance_unclosed_brackets(r#"{"a":1"#).as_deref(),
        Some(r#"{"a":1}"#)
    );
    // 嵌套缺多个 `}`。
    assert_eq!(
        super::balance_unclosed_brackets(r#"{"a":{"b":{"c":1"#).as_deref(),
        Some(r#"{"a":{"b":{"c":1}}}"#)
    );
    // 字符串里出现 `{` / `}` 不应当成结构标记。
    assert!(super::balance_unclosed_brackets(r#"{"text":"{x}"}"#).is_none());
    // 数组也兼容。
    assert_eq!(
        super::balance_unclosed_brackets(r#"[1,[2,3"#).as_deref(),
        Some(r#"[1,[2,3]]"#)
    );
    // 字符串未闭合 + 缺 `}`：先补 `"`，再补 `}`。
    assert_eq!(
        super::balance_unclosed_brackets(r#"{"a":"hello"#).as_deref(),
        Some(r#"{"a":"hello"}"#)
    );
}

/// §F3-a：adv12 复现的 MiniMax 输出模式（结尾少一个 `}` +
/// 废弃/未知字段误嵌入 `execution_recipe`）必须能被 repair 成可解析。
#[test]
fn parse_llm_json_raw_or_any_with_repair_recovers_adv12_minimax_envelope() {
    // 注意：原始 JSON 末尾少了 envelope 自己的最后一个 `}`，
    // 且废弃字段错误地嵌入 `execution_recipe`。repair 后应能解析并保留
    // envelope 顶层字段。
    let raw = r#"{"resolved_user_intent":"x","resume_behavior":"none","schedule_kind":"none","schedule_intent":null,"wants_file_delivery":false,"should_refresh_long_term_memory":false,"agent_display_name_hint":"","needs_clarify":false,"clarify_question":"","reason":"r","confidence":0.95,"decision":"planner_execute","output_contract":{"response_shape":"free","requires_content_evidence":false,"delivery_required":false,"locator_kind":"filename","delivery_intent":"none","semantic_kind":"existence_with_path","locator_hint":"AGENTS.md","self_extension":{"mode":"none","trigger":"none","execute_now":false}},"execution_recipe":{"kind":"none","profile":"none","target_scope":"current_repo","unknown_extra_text":"","unknown_extra_score":0.0}"#;
    // 直接 from_str 必失败：少最后一个 `}`。
    assert!(serde_json::from_str::<serde_json::Value>(raw).is_err());
    let parsed = super::parse_llm_json_raw_or_any_with_repair::<serde_json::Value>(raw)
        .expect("balance pass should recover truncated MiniMax envelope");
    assert_eq!(
        parsed.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute"),
        "envelope decision field must survive repair"
    );
    assert_eq!(
        parsed.get("needs_clarify").and_then(|v| v.as_bool()),
        Some(false),
        "envelope needs_clarify must survive repair"
    );
    assert_eq!(
        parsed
            .pointer("/output_contract/locator_kind")
            .and_then(|v| v.as_str()),
        Some("filename")
    );
}

#[test]
fn schedule_intent_schema_canonicalizes_normalizer_envelope() {
    let raw = r#"{
      "resolved_user_intent":"Create a daily reminder in the current conversation.",
      "resume_behavior":"none",
      "schedule_kind":"create",
      "schedule_intent":{
        "timezone":"Asia/Shanghai",
        "schedule":{"type":"daily","run_at":"","time":"08:00","weekday":1,"every_minutes":0,"cron":""},
        "message":"daily reminder message"
      },
      "needs_clarify":false,
      "clarify_question":"",
      "reason":"schedule fields are complete",
      "confidence":0.93
    }"#;
    let validated = super::validate_against_schema::<crate::ScheduleIntentOutput>(
        raw,
        super::PromptSchemaId::ScheduleIntent,
    )
    .expect("schedule intent envelope should canonicalize")
    .value;

    assert_eq!(validated.kind, "create");
    assert_eq!(validated.target_job_id, "");
    assert_eq!(
        validated.raw,
        "Create a daily reminder in the current conversation."
    );
    assert_eq!(validated.task.kind, "ask");
    assert_eq!(
        validated
            .task
            .payload
            .get("message")
            .and_then(|value| value.as_str()),
        Some("daily reminder message")
    );
}

#[test]
fn parse_llm_json_raw_or_any_with_repair_keeps_valid_json() {
    let raw = r#"{"decision":"direct_answer","confidence":0.9}"#;
    let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
        .expect("valid json should parse");
    assert_eq!(
        parsed
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
        "direct_answer"
    );
}

#[test]
fn parse_llm_json_raw_or_any_with_repair_removes_stray_quote_after_bool() {
    let raw = r#"{"decision":"planner_execute","needs_clarify":false","confidence":0.9}"#;
    assert!(serde_json::from_str::<Value>(raw).is_err());
    let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
        .expect("stray quote after primitive should repair");
    assert_eq!(
        parsed.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        parsed.get("needs_clarify").and_then(|v| v.as_bool()),
        Some(false)
    );
}

/// §D1：dedupe_json_object_keys 幂等性。任意 JSON dedup 一次和二次结果必须一致。
/// 防止未来引入「dedup 自身搬动了 key 顺序导致再 dedup 又改」这种回归。
#[test]
fn dedupe_json_object_keys_is_idempotent() {
    let corpus = [
        r#"{"a":1}"#,
        r#"{"a":1,"a":2}"#,
        r#"{"a":1,"a":2,"a":3,"a":4}"#,
        r#"{"a":{"b":1,"b":2},"a":{"b":3,"b":4}}"#,
        r#"[{"x":1,"x":2},{"x":3,"x":4}]"#,
        r#"{"decision":"planner_execute","execution_recipe":{"kind":"none","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#,
        r#"{"a":[1,2,3],"a":[4,5,6]}"#,
        r#"{}"#,
        r#"[]"#,
        r#""hi""#,
        r#"42"#,
        r#"true"#,
        r#"null"#,
    ];
    for raw in corpus {
        let once = super::dedupe_json_object_keys(raw).expect("first dedup pass should succeed");
        let twice =
            super::dedupe_json_object_keys(&once).expect("second dedup pass should succeed");
        assert_eq!(
            once, twice,
            "dedupe_json_object_keys not idempotent on input {}",
            raw
        );
    }
}

/// §D1：N-fold 重复键 last-wins 规则覆盖。覆盖兼容模型偶发把同一字段
/// 重复 2/3/5/10 次的全部观测形态。
#[test]
fn dedupe_json_object_keys_last_wins_for_n_fold_duplicates() {
    for n in [2usize, 3, 5, 10] {
        let mut payload = String::from("{");
        for i in 0..n {
            if i > 0 {
                payload.push(',');
            }
            payload.push_str(&format!(r#""x":"v{}""#, i));
        }
        payload.push('}');
        let deduped = super::dedupe_json_object_keys(&payload)
            .expect("n-fold duplicate input should round-trip through Value");
        let parsed: Value =
            serde_json::from_str(&deduped).expect("dedup output should still parse as Value");
        assert_eq!(
            parsed.get("x").and_then(|v| v.as_str()),
            Some(format!("v{}", n - 1).as_str()),
            "expected last-wins for n={}, got: {}",
            n,
            deduped
        );
    }
}

/// §D1：minimax 实际观测的「病态 JSON 语料库」全部能跑通解析回路 —— 含
/// duplicate keys / 嵌套 duplicate / 数组里的 object-with-duplicates / 数值与
/// bool 重复 / null 与字符串混合重复。任何一条 panic 都视为回归。
///
/// 这里**不**断言每一条都能 dedup 成功；只断言不 panic 且能 round-trip：
/// `parse_llm_json_raw_or_any_with_repair::<Value>(...)` 拿到结果后再 to_string
/// 然后再 dedup 仍然能 parse。
#[test]
fn parse_llm_json_raw_or_any_with_repair_survives_minimax_pathological_corpus() {
    let corpus = [
        // duplicate top-level keys
        r#"{"target_scope":"system","target_scope":"system"}"#,
        // duplicate top + duplicate nested
        r#"{"a":"x","a":"y","b":{"c":1,"c":2,"c":3}}"#,
        // duplicate inside array element
        r#"{"items":[{"k":1,"k":2},{"k":3,"k":4,"k":5}]}"#,
        // duplicate boolean / null mixed
        r#"{"flag":true,"flag":false,"missing":null,"missing":"present"}"#,
        // duplicate keys with mixed value types (str -> obj)
        r#"{"contract":"loose","contract":{"shape":"strict"}}"#,
        // realistic minimax normalizer payload: duplicate target_scope inside
        // execution_recipe nested in IntentNormalizerOut-style envelope.
        r#"{"resolved_user_intent":"check service","decision":"planner_execute","needs_clarify":false,"reason":"r","confidence":0.8,"execution_recipe":{"kind":"ops_closed_loop","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#,
    ];
    for raw in corpus {
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .unwrap_or_else(|| panic!("failed to repair-and-parse: {}", raw));
        let reserialized =
            serde_json::to_string(&parsed).expect("repaired Value should re-serialize");
        let again = super::parse_llm_json_raw_or_any_with_repair::<Value>(&reserialized)
            .unwrap_or_else(|| panic!("re-parse of normalized form failed: {}", reserialized));
        assert!(
            again.is_object()
                || again.is_array()
                || again.is_string()
                || again.is_number()
                || again.is_boolean()
                || again.is_null()
        );
    }
}

#[test]
fn extract_agent_action_objects_recovers_inner_actions_from_malformed_wrapper() {
    let raw = r#"{"steps":[{"type":"call_skill","skill":"read_file","args":{"path":"README.md"}},{"type":"call_skill","skill":"system_basic","args":{"action":"info"}]}"#;
    let extracted = super::extract_agent_action_objects(raw);
    assert_eq!(extracted.len(), 2);
    let parsed: Value =
        serde_json::from_str(&extracted[0]).expect("first inner action should parse");
    assert_eq!(
        parsed.get("skill").and_then(|v| v.as_str()),
        Some("read_file")
    );
    let parsed_second: Value =
        serde_json::from_str(&extracted[1]).expect("second inner action should parse");
    assert_eq!(
        parsed_second.get("skill").and_then(|v| v.as_str()),
        Some("system_basic")
    );
}

#[test]
fn normalize_agent_action_shape_rewrites_bare_run_cmd_aliases() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"run_cmd","cmd":"pwd","workdir":"/tmp","timeout_ms":2500}"#,
        &state,
    )
    .expect("bare run_cmd should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "run_cmd",
            "args": {
                "command": "pwd",
                "cwd": "/tmp",
                "timeout_seconds": 3
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_preserves_bare_run_cmd_args_object() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"run_cmd","args":{"command":"git branch --show-current","cwd":"/tmp/repo"}}"#,
        &state,
    )
    .expect("bare run_cmd args object should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "run_cmd",
            "args": {
                "command": "git branch --show-current",
                "cwd": "/tmp/repo"
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_preserves_internal_run_cmd_metadata() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"call_skill","skill":"run_cmd","args":{"command":"bash /tmp/check.sh","cwd":"/tmp","_clawd_validation":{"profile":"code_change","validator_type":"runtime_probe","validated_target":"/tmp/check.sh"}}}"#,
        &state,
    )
    .expect("run_cmd should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "run_cmd",
            "args": {
                "command": "bash /tmp/check.sh",
                "cwd": "/tmp",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "runtime_probe",
                    "validated_target": "/tmp/check.sh"
                }
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_preserves_top_level_internal_run_cmd_metadata() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"run_cmd","cmd":"pwd","_clawd_continue_on_error":true}"#,
        &state,
    )
    .expect("bare run_cmd should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "run_cmd",
            "args": {
                "command": "pwd",
                "_clawd_continue_on_error": true
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_rewrites_action_run_cmd_alias() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"action":"run_cmd","cmd":"pwd","workdir":"/tmp"}"#,
        &state,
    )
    .expect("action run_cmd should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "run_cmd",
            "args": {
                "command": "pwd",
                "cwd": "/tmp"
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_rewrites_action_builtin_skill_alias() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"action":"list_dir","path":"logs","limit":2}"#,
        &state,
    )
    .expect("action builtin skill should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "list_dir",
            "args": {
                "path": "logs",
                "limit": 2
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_rewrites_system_basic_run_cmd_to_run_cmd_skill() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"call_skill","skill":"system_basic","args":{"action":"run_cmd","command":"git branch --show-current","description":"获取当前git分支名称"}}"#,
        &state,
    )
    .expect("system_basic run_cmd should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "run_cmd",
            "args": {
                "command": "git branch --show-current"
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_rewrites_call_tool_run_cmd_aliases() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"call_tool","tool":"run_cmd","args":{"cmd":"whoami","timeout_ms":1}}"#,
        &state,
    )
    .expect("call_tool run_cmd should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "run_cmd",
            "args": {
                "command": "whoami",
                "timeout_seconds": 1
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_preserves_call_tool_fs_basic_write_text() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"call_tool","tool":"fs_basic","args":{"action":"write_text","path":"/tmp/path_note.txt","content":"hello"}}"#,
        &state,
    )
    .expect("call_tool fs_basic should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_tool",
            "tool": "fs_basic",
            "args": {
                "action": "write_text",
                "path": "/tmp/path_note.txt",
                "content": "hello"
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_rewrites_system_basic_list_dir_to_base_skill() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"call_skill","skill":"system_basic","args":{"action":"list_dir","path":"scripts","names_only":true}}"#,
        &state,
    )
    .expect("system_basic list_dir should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "list_dir",
            "args": {
                "path": "scripts",
                "names_only": true
            }
        })
    );
}

#[test]
fn normalize_agent_action_shape_rewrites_rich_system_basic_list_dir_to_inventory_dir() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let normalized = super::parse_agent_action_json_with_repair(
        r#"{"type":"call_skill","skill":"system_basic","args":{"action":"list_dir","path":"logs","sort_by":"mtime","limit":2,"names_only":true,"options":{"show_timestamps":true}}}"#,
        &state,
    )
    .expect("rich system_basic list_dir should normalize");
    assert_eq!(
        normalized,
        json!({
            "type": "call_skill",
            "skill": "system_basic",
            "args": {
                "action": "inventory_dir",
                "path": "logs",
                "sort_by": "mtime_desc",
                "max_entries": 2,
                "names_only": true
            }
        })
    );
}
