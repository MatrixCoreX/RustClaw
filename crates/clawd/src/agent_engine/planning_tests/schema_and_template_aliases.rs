use super::*;

#[test]
fn rewrite_terminal_step_output_alias_placeholder_inserts_synthesize_answer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "docs"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": "docs/release_checklist.md"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{step1_output}} and {{step3_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 5);
    assert!(matches!(
        &rewritten[3],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice() == ["step_1".to_string(), "step_3".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[4],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn rewrite_terminal_placeholder_preserves_mixed_last_output_respond() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pwd"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}\n\n这个路径是当前工作目录，通常对应正在操作的项目根目录。"
                .to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content }
            if content.contains("{{last_output}}") && content.contains("当前工作目录")
    ));
}

#[test]
fn unresolved_template_arg_multi_file_read_plan_uses_direct_file_reads() {
    let route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": "README.md", "mode": "head", "n": 40}),
        },
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({"action": "find_name", "name": "AGENTS.md"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": "{{s1_match}}", "mode": "head", "n": 40}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s0".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_unresolved_template_arg_multi_file_read_plan(
        Some(&route),
        "read the opening section of README.md, then read the opening section of AGENTS.md",
        actions,
    );

    assert_eq!(rewritten.len(), 4);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("AGENTS.md")
    ));
    assert!(matches!(
        &rewritten[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice() == ["step_1".to_string(), "step_2".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

/// §D2.a：plan_result schema 与 `AgentAction` enum / `SinglePlanEnvelope` 漂移检查。
///
/// 校验内容：
/// 1. `prompts/schemas/plan_result.schema.json` 是合法 JSON 且为 object schema；
/// 2. envelope 顶层 required 含 `steps`；
/// 3. `$defs/AgentAction.oneOf` 必须正好覆盖 6 个 variant：think / call_skill /
///    call_tool / call_capability / synthesize_answer / respond（与 `AgentAction` enum 一一对应）；
/// 4. 每个 variant 的 `type` const 必须是 snake_case 的 variant 名；
/// 5. 每个 variant 的 required 字段必须 ⊇ `AgentAction` 该 variant 的非空字段；
/// 6. 完整性闭环：把每个 variant 的最小合法实例 round-trip
///    `serde_json::from_value::<AgentAction>` 必须成功。
#[test]
fn plan_result_schema_drift() {
    const SCHEMA_RAW: &str = include_str!("../../../../../prompts/schemas/plan_result.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("plan_result.schema.json must be valid JSON");
    assert_eq!(
        schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "schema root must be object"
    );
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&json!(false)),
        "schema root must reject unknown envelope fields after canonicalization"
    );
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema must have `required`");
    assert!(
        required.iter().any(|v| v.as_str() == Some("steps")),
        "envelope must require `steps`"
    );
    let defs = schema
        .get("$defs")
        .and_then(|v| v.as_object())
        .expect("schema must declare $defs");
    let action = defs
        .get("AgentAction")
        .expect("$defs.AgentAction must exist");
    let one_of = action
        .get("oneOf")
        .and_then(|v| v.as_array())
        .expect("AgentAction must be a oneOf union");

    // 期望与 `AgentAction` enum 完全对齐：think / call_skill / call_tool /
    // call_capability / synthesize_answer / respond
    let expected: HashSet<&str> = [
        "think",
        "call_skill",
        "call_tool",
        "call_capability",
        "synthesize_answer",
        "respond",
    ]
    .into_iter()
    .collect();
    let mut actual: HashSet<String> = HashSet::new();
    for entry in one_of {
        let ref_path = entry
            .get("$ref")
            .and_then(|v| v.as_str())
            .expect("oneOf entry must use $ref");
        let def_name = ref_path
            .strip_prefix("#/$defs/")
            .expect("$ref must point under #/$defs/");
        let def = defs.get(def_name).expect("referenced def must exist");
        assert_eq!(
            def.get("additionalProperties"),
            Some(&json!(false)),
            "variant `{}` must reject unknown action fields after canonicalization",
            def_name
        );
        let type_const = def
            .get("properties")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.get("const"))
            .and_then(|v| v.as_str())
            .expect("variant must declare `properties.type.const`");
        actual.insert(type_const.to_string());
    }
    let actual_refs: HashSet<&str> = actual.iter().map(String::as_str).collect();
    assert_eq!(
        actual_refs, expected,
        "plan_result.schema.json AgentAction oneOf must cover exactly {:?}, got {:?}",
        expected, actual_refs
    );

    // §D2.a 步骤 6：每个 variant 的最小合法实例必须能反序列化进 AgentAction。
    let probes: &[(&str, serde_json::Value)] = &[
        ("think", json!({"type": "think", "content": "x"})),
        (
            "call_skill",
            json!({"type": "call_skill", "skill": "run_cmd", "args": {}}),
        ),
        (
            "call_tool",
            json!({"type": "call_tool", "tool": "read_file", "args": {}}),
        ),
        (
            "call_capability",
            json!({"type": "call_capability", "capability": "filesystem.list_entries", "args": {}}),
        ),
        (
            "synthesize_answer",
            json!({"type": "synthesize_answer", "evidence_refs": ["last_output"]}),
        ),
        ("respond", json!({"type": "respond", "content": "ok"})),
    ];
    for (label, value) in probes {
        serde_json::from_value::<AgentAction>(value.clone()).unwrap_or_else(|err| {
                panic!(
                    "AgentAction variant `{}` failed to deserialize from schema-conformant minimum payload: {}",
                    label, err
                )
            });
    }
}

#[tokio::test]
async fn parse_single_plan_accepts_respond_only_step_with_top_level_content() {
    let state = test_state_with_registry();
    let task = test_task();
    let raw = r#"{
      "steps": [
        {
          "type": "respond",
          "content": "【面向老板的方案模板】\n\n一、背景与机会\n| 风险 | 概率 |\n|---|---|"
        }
      ]
    }"#;

    let actions = super::super::parse_single_plan_actions(raw, &state, &task)
        .await
        .expect("respond-only plan should parse");

    assert!(matches!(
        actions.as_slice(),
        [AgentAction::Respond { content }]
            if content.contains("面向老板") && content.contains("|---|")
    ));
}

#[tokio::test]
async fn parse_single_plan_recovers_malformed_respond_step_with_extra_closer() {
    let state = test_state_with_registry();
    let task = test_task();
    let raw = r#"{
      "steps": [
        {
          "type": "respond",
          "content": "【面向老板的方案模板】\n\n一、背景与机会\n| 风险 | 概率 |\n|---|---|"
        }}
      ]
    }"#;

    let actions = super::super::parse_single_plan_actions(raw, &state, &task)
        .await
        .expect("malformed respond-only plan should recover");

    assert!(matches!(
        actions.as_slice(),
        [AgentAction::Respond { content }]
            if content.contains("面向老板") && content.contains("|---|")
    ));
}

#[tokio::test]
async fn parse_single_plan_recovers_terminal_steps_with_multiline_respond_content() {
    let state = test_state_with_registry();
    let task = test_task();
    let raw = r#"{"steps":[{"type":"synthesize_answer","evidence_refs":["s1"]},{"type":"respond","content":"logs directory entries:
1. act_plan.log
2. agent_rollout_metrics
3. agent_rollout_metrics.zip
4. base_skill_contracts_20260516_100540"}]}"#;

    let actions = super::super::parse_single_plan_actions(raw, &state, &task)
        .await
        .expect("terminal plan with malformed multiline respond content should recover");

    assert!(matches!(
        actions.as_slice(),
        [
            AgentAction::SynthesizeAnswer { evidence_refs },
            AgentAction::Respond { content }
        ] if evidence_refs.as_slice() == ["s1".to_string()].as_slice()
            && content.contains("act_plan.log")
            && content.contains("base_skill_contracts_20260516_100540")
    ));
}

#[tokio::test]
async fn parse_single_plan_accepts_synthesize_answer_only_step_with_top_level_refs() {
    let state = test_state_with_registry();
    let task = test_task();
    let raw = r#"{
      "steps": [
        {
          "type": "synthesize_answer",
          "evidence_refs": ["last_output"]
        }
      ]
    }"#;

    let actions = super::super::parse_single_plan_actions(raw, &state, &task)
        .await
        .expect("synthesize-answer-only plan should parse");

    assert!(matches!(
        actions.as_slice(),
        [AgentAction::SynthesizeAnswer { evidence_refs }]
            if evidence_refs.as_slice() == ["last_output".to_string()].as_slice()
    ));
}

#[tokio::test]
async fn parse_single_plan_normalizes_terminal_call_tool_wrappers() {
    let state = test_state_with_registry();
    let task = test_task();
    let raw = r#"{
      "steps": [
        {
          "type": "call_tool",
          "tool": "fs_basic",
          "args": {
            "action": "write_text",
            "path": "calc_core.py",
            "content": "def add(a, b):\n    return a + b\n"
          }
        },
        {
          "type": "call_tool",
          "tool": "run_cmd",
          "args": {
            "command": "python3 test_calc_core.py"
          }
        },
        {
          "type": "call_tool",
          "tool": "synthesize_answer",
          "args": {
            "evidence_refs": ["step_1", "step_2"]
          }
        },
        {
          "type": "call_tool",
          "tool": "respond",
          "args": {
            "content": "{{last_output}}"
          }
        }
      ]
    }"#;

    let actions = super::super::parse_single_plan_actions(raw, &state, &task)
        .await
        .expect("terminal call_tool wrappers should parse");

    assert!(matches!(
        actions.as_slice(),
        [
            AgentAction::CallTool { tool, .. },
            AgentAction::CallSkill { skill, .. },
            AgentAction::SynthesizeAnswer { evidence_refs },
            AgentAction::Respond { content },
        ] if tool == "fs_basic"
            && skill == "run_cmd"
            && evidence_refs.as_slice() == ["step_1".to_string(), "step_2".to_string()].as_slice()
            && content == "{{last_output}}"
    ));
}

#[tokio::test]
async fn parse_single_plan_accepts_synthesize_answer_refs_inside_args() {
    let state = test_state_with_registry();
    let task = test_task();
    let raw = r#"{
      "steps": [
        {
          "type": "synthesize_answer",
          "args": {
            "evidence_refs": ["step_2"]
          }
        }
      ]
    }"#;

    let actions = super::super::parse_single_plan_actions(raw, &state, &task)
        .await
        .expect("synthesize_answer args wrapper should parse");

    assert!(matches!(
        actions.as_slice(),
        [AgentAction::SynthesizeAnswer { evidence_refs }]
            if evidence_refs.as_slice() == ["step_2".to_string()].as_slice()
    ));
}

fn safe_div_agent_loop_execution_actions() -> Vec<AgentAction> {
    vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new/calc_core.py"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new/calc_core.py",
                "content": "def safe_div(a, b):\n    if b == 0:\n        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n    return {\"ok\": True, \"value\": a / b}\n"
            }),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({
                "command": "python3 test_calc_core.py",
                "cwd": "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new"
            }),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({
                "command": "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1, 0))\nPY",
                "cwd": "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "last_output".to_string(),
                "s3".to_string(),
                "s4".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ]
}

#[test]
fn agent_loop_execution_code_plan_defers_to_verifier_without_pre_repair() {
    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.risk_ceiling = RiskCeiling::Medium;
    route.route_reason = "inline_structured_payload_preserved_as_execution_spec; executable_contract_preserved_for_agent_loop; execution_recipe_target_locator_preserved_for_agent_loop".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    let loop_state = LoopState::new(1);
    let actions = safe_div_agent_loop_execution_actions();

    assert!(
        !should_force_actionable_plan_repair(&state, Some(&route), &loop_state, &actions),
        "agent-loop execution plan should reach verifier/loop without LLM repair"
    );
}

#[test]
fn agent_loop_execution_code_plan_can_fallback_when_repair_aborts() {
    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.risk_ceiling = RiskCeiling::Medium;
    route.route_reason = "inline_structured_payload_preserved_as_execution_spec; executable_contract_preserved_for_agent_loop; execution_recipe_target_locator_preserved_for_agent_loop".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    let loop_state = LoopState::new(1);
    let actions = safe_div_agent_loop_execution_actions();

    assert!(
        can_fallback_to_initial_plan_after_repair_failure(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ),
        "valid initial agent-loop execution plan should survive repair-model aborts"
    );
}
