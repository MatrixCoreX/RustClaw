use super::*;

#[test]
fn directory_purpose_summary_from_size_facts_picks_largest_file() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"names":["contract_repair_judge.schema.json","intent_normalizer.schema.json"]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/contract_repair_judge.schema.json","size_bytes":6112},"path":"prompts/schemas/contract_repair_judge.schema.json"},{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":13124},"path":"prompts/schemas/intent_normalizer.schema.json"}]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "prompts/schemas contains JSON Schema contracts; intent_normalizer.schema.json is 13124 bytes and describes the structured intent-normalizer output.",
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "prompts/schemas".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_directory_purpose_summary_from_size_facts(
        &state,
        "告诉我哪个 schema 最大",
        &loop_state,
        Some(&ctx),
    )
    .expect("directory purpose size facts answer");

    assert!(answer.contains("intent_normalizer.schema.json"));
    assert!(answer.contains("13124"));
    assert!(answer.contains("directory_purpose_summary="));
    assert!(answer.contains("intent-normalizer"));
    assert!(!answer.contains("contract_repair_judge.schema.json（6112"));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn directory_purpose_summary_replaces_wrong_synthesis_largest_file() {
    let state = test_state();
    let task = claimed_task("task-directory-purpose-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("最大的是 contract_repair_judge.schema.json（6112 字节）。".to_string());
    loop_state.last_user_visible_respond =
        Some("最大的是 contract_repair_judge.schema.json（6112 字节）。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/contract_repair_judge.schema.json","size_bytes":6112},"path":"prompts/schemas/contract_repair_judge.schema.json"},{"exists":true,"fact":{"kind":"file","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":13124},"path":"prompts/schemas/intent_normalizer.schema.json"}]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "prompts/schemas keeps schema contracts; intent_normalizer.schema.json is the largest observed schema at 13124 bytes and describes intent-normalizer output.",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "contract_repair_judge.schema.json looks like the largest file.",
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "prompts/schemas".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_directory_purpose_answer(
            &state,
            &task,
            "告诉我哪个 schema 最大",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("intent_normalizer.schema.json"));
    assert!(answer.contains("13124"));
    assert!(answer.contains("directory_purpose_summary="));
    assert!(answer.contains("intent-normalizer output"));
    assert!(!answer.contains("contract_repair_judge.schema.json（6112"));
    assert!(summary.is_some());
}

#[test]
fn directory_purpose_summary_preserves_latest_publishable_synthesis_delivery() {
    let state = test_state();
    let task = claimed_task("task-directory-purpose-preserve-synthesis");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":3,"hidden":0,"total":3},"entries":[{"kind":"file","name":"answer_verifier.schema.json","path":"prompts/schemas/answer_verifier.schema.json","size_bytes":1478},{"kind":"file","name":"contract_repair_judge.schema.json","path":"prompts/schemas/contract_repair_judge.schema.json","size_bytes":6754},{"kind":"file","name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":14775}],"names":["answer_verifier.schema.json","contract_repair_judge.schema.json","intent_normalizer.schema.json"],"size_summary":{"largest_file":{"kind":"file","name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":14775}}}}"#,
    ));
    let synthesis = "prompts/schemas/ 目录下共有 3 个 .json schema 文件，其中体积最大的是 `intent_normalizer.schema.json`（14775 字节）。这个文件定义了意图归一化器的输出结构。";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state.delivery_messages.push(synthesis.to_string());
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "prompts/schemas".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_directory_purpose_answer(
            &state,
            &task,
            "概览 prompts/schemas",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![synthesis.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesis)
    );
    assert!(summary.is_none());
}

#[test]
fn directory_purpose_summary_replaces_wrapped_inventory_wrong_synthesis_with_compact_fields() {
    let state = test_state();
    let task = claimed_task("task-directory-purpose-wrapped-inventory");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("contract_repair_judge.schema.json looks like the largest file.".to_string());
    loop_state.last_user_visible_respond =
        Some("contract_repair_judge.schema.json looks like the largest file.".to_string());
    let inventory = serde_json::json!({
        "extra": {
            "action": "inventory_dir",
            "counts": {"dirs": 0, "files": 3, "hidden": 0, "total": 3},
            "entries": [
                {"kind": "file", "name": "answer_verifier.schema.json", "path": "prompts/schemas/answer_verifier.schema.json", "size_bytes": 1478},
                {"kind": "file", "name": "contract_repair_judge.schema.json", "path": "prompts/schemas/contract_repair_judge.schema.json", "size_bytes": 6754},
                {"kind": "file", "name": "intent_normalizer.schema.json", "path": "prompts/schemas/intent_normalizer.schema.json", "size_bytes": 14775}
            ],
            "names": [
                "answer_verifier.schema.json",
                "contract_repair_judge.schema.json",
                "intent_normalizer.schema.json"
            ],
            "size_summary": {
                "largest_file": {
                    "kind": "file",
                    "name": "intent_normalizer.schema.json",
                    "path": "prompts/schemas/intent_normalizer.schema.json",
                    "size_bytes": 14775
                }
            }
        },
        "text": "{\"action\":\"inventory_dir\"}"
    })
    .to_string();
    let read_range = serde_json::json!({
        "extra": {
            "action": "read_range",
            "path": "prompts/schemas/intent_normalizer.schema.json",
            "resolved_path": "/repo/prompts/schemas/intent_normalizer.schema.json",
            "excerpt": "1|{\n2|  \"title\": \"IntentNormalizerOut\",\n3|  \"description\": \"Schema for the JSON object returned by the unified intent normalizer prompt.\""
        },
        "text": "{\"action\":\"read_range\"}"
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &inventory));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "fs_basic", &read_range));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "contract_repair_judge.schema.json looks like the largest file.",
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "prompts/schemas".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_directory_purpose_answer(
            &state,
            &task,
            "列出 schemas 并说明最大文件",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.lines().count() <= 5);
    assert!(answer.contains("documentation.files.count=3"));
    assert!(answer.contains("intent_normalizer.schema.json"));
    assert!(answer.contains("largest.size_bytes=14775"));
    assert!(answer.contains("content_excerpt="));
    assert!(answer.contains("IntentNormalizerOut"));
    assert!(!answer.contains("contract_repair_judge.schema.json looks like the largest file"));
    assert!(summary.is_some());
}

#[test]
fn directory_purpose_summary_replaces_partial_document_listing_synthesis() {
    let state = test_state();
    let task = claimed_task("task-directory-purpose-partial-listing");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    let partial = "docs 目录包含 Base Tool Capability Matrix、config_basic Contract、fs_basic Contract 和 Planning Deterministic Guardrails Audit 四个文档。";
    loop_state.delivery_messages.push(partial.to_string());
    loop_state.last_user_visible_respond = Some(partial.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":5,"hidden":0,"total":5},"entries":[{"kind":"file","name":"base_skill_response_contract.md","path":"docs/base_skill_response_contract.md","size_bytes":2463},{"kind":"file","name":"base_tool_capability_matrix.md","path":"docs/base_tool_capability_matrix.md","size_bytes":5087},{"kind":"file","name":"config_basic_contract.md","path":"docs/config_basic_contract.md","size_bytes":3559},{"kind":"file","name":"fs_basic_contract.md","path":"docs/fs_basic_contract.md","size_bytes":4555},{"kind":"file","name":"planning_deterministic_guardrails_audit.md","path":"docs/planning_deterministic_guardrails_audit.md","size_bytes":3400}],"names":["base_skill_response_contract.md","base_tool_capability_matrix.md","config_basic_contract.md","fs_basic_contract.md","planning_deterministic_guardrails_audit.md"],"path":"/home/guagua/rustclaw/docs"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "docs contains RustClaw contract documentation; base_tool_capability_matrix.md is the largest observed document at 5087 bytes and anchors the capability matrix overview.",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "planning_deterministic_guardrails_audit.md is the main document.",
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "docs".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_directory_purpose_answer(
            &state,
            &task,
            "看看 docs 目录，先基于目录内容给我一个简短概览",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("documentation.files.count=5"));
    assert!(answer.contains("base_skill_response_contract.md"));
    assert!(answer.contains("planning_deterministic_guardrails_audit.md"));
    assert!(answer.contains("directory_purpose_summary="));
    assert!(answer.contains("capability matrix overview"));
    assert!(!answer.contains("四个文档"));
    assert!(summary.is_some());
}

#[test]
fn directory_purpose_summary_uses_listing_content_when_synthesis_left_no_delivery() {
    let state = test_state();
    let task = claimed_task("task-directory-purpose-empty-delivery");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"kind":"file","name":"base_skill_response_contract.md","path":"docs/base_skill_response_contract.md","size_bytes":2463},{"kind":"file","name":"base_tool_capability_matrix.md","path":"docs/base_tool_capability_matrix.md","size_bytes":5087}],"names":["base_skill_response_contract.md","base_tool_capability_matrix.md"],"path":"/home/guagua/rustclaw/docs"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"docs/base_tool_capability_matrix.md","resolved_path":"/home/guagua/rustclaw/docs/base_tool_capability_matrix.md","excerpt":"1|# Base Tool Capability Matrix\n2|This document describes the planner-facing capability matrix."}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "docs".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_directory_purpose_answer(
            &state,
            &task,
            "看看 docs 目录，先基于目录内容给我一个简短概览",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("documentation.files.count=2"));
    assert!(answer.contains("base_skill_response_contract.md"));
    assert!(answer.contains("base_tool_capability_matrix.md"));
    assert!(summary.is_some());
}

#[test]
fn directory_purpose_summary_does_not_finalize_listing_only_intermediate_state() {
    let state = test_state();
    let task = claimed_task("task-directory-purpose-listing-only");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"kind":"file","name":"contract_repair_judge.schema.json","path":"prompts/schemas/contract_repair_judge.schema.json","size_bytes":6754},{"kind":"file","name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":14775}],"names":["contract_repair_judge.schema.json","intent_normalizer.schema.json"],"path":"/home/guagua/rustclaw/prompts/schemas"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "I need to read the largest schema before answering.",
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "prompts/schemas".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        !replace_delivery_with_deterministic_directory_purpose_answer(
            &state,
            &task,
            "列出 schemas 并说明最大文件",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );
    assert!(summary.is_none());
}

#[test]
fn current_workspace_dirs_overview_reads_wrapped_inventory_dir_output() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":2,\"files\":1,\"total\":3},\"names_by_kind\":{\"dirs\":[\"configs\",\"docs\"],\"files\":[\"README.md\"],\"other\":[]},\"path\":\"workspace\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"dirs":["configs","docs"],"files":["README.md"],"other":[]},"path":"workspace"}}"#,
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_current_workspace_top_level_dirs_overview_answer(
        &state,
        "先看看当前目录有哪些顶层文件夹，再用一句适合新手的话解释这个仓库大概怎么组织",
        &loop_state,
        Some(&ctx),
    )
    .expect("current workspace dirs overview answer");

    assert!(answer.contains("configs"));
    assert!(answer.contains("docs"));
    assert!(answer.contains("field_value.workspace.top_level.dirs.count=2"));
    assert!(answer.contains("field_value.workspace.top_level.files=README.md"));
    assert!(answer.contains("workspace.top_level.dirs.count=2"));
    assert!(answer.contains("workspace.top_level.files.count=1"));
    assert!(answer.contains("- README.md"));
    assert!(answer.contains("workspace.overview.kind=repository_sections_by_purpose"));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn current_workspace_inventory_overview_supports_free_listing_contract() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":3,"total":5},"names_by_kind":{"dirs":["configs","docs"],"files":["AGENTS.md","Cargo.toml","README.md"],"other":[]},"path":"."}"#,
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_current_workspace_top_level_dirs_overview_answer(
        &state,
        "group the top-level entries",
        &loop_state,
        Some(&ctx),
    )
    .expect("free current workspace inventory answer");

    assert!(answer.contains("workspace.top_level.dirs.count=2"));
    assert!(answer.contains("field_value.workspace.top_level.dirs=configs,docs"));
    assert!(answer.contains("- configs"));
    assert!(answer.contains("- docs"));
    assert!(answer.contains("workspace.top_level.files.count=3"));
    assert!(answer.contains("field_value.workspace.top_level.files=AGENTS.md,Cargo.toml,README.md"));
    assert!(answer.contains("- AGENTS.md"));
    assert!(answer.contains("- Cargo.toml"));
    assert!(answer.contains("- README.md"));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn current_workspace_dirs_overview_replaces_incomplete_generic_synthesis() {
    let state = test_state();
    let task = claimed_task("task-current-workspace-dirs-overview");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":4,\"files\":1,\"total\":5},\"names_by_kind\":{\"dirs\":[\"UI\",\"configs\",\"crates\",\"docs\"],\"files\":[\"README.md\"],\"other\":[]},\"path\":\"workspace\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":4,"files":1,"total":5},"names_by_kind":{"dirs":["UI","configs","crates","docs"],"files":["README.md"],"other":[]},"path":"workspace"}}"#,
    ));
    let incomplete = "这是一个标准的 Rust 项目，根目录包含 UI、configs、crates 等目录。";
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        incomplete,
    ));
    loop_state.delivery_messages.push(incomplete.to_string());
    loop_state.last_user_visible_respond = Some(incomplete.to_string());
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_current_workspace_dirs_overview_answer(
            &state,
            &task,
            "先看看当前目录有哪些顶层文件夹，再用一句适合新手的话解释这个仓库大概怎么组织",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("workspace.top_level.dirs.count=4"));
    assert!(answer.contains("workspace.top_level.files.count=1"));
    assert!(answer.contains("UI"));
    assert!(answer.contains("configs"));
    assert!(answer.contains("crates"));
    assert!(answer.contains("docs"));
    assert!(answer.contains("README.md"));
    assert!(answer.contains("workspace.overview.section_hints="));
    assert!(!answer.contains("标准的 Rust 项目"));
    assert!(summary.is_some());
}

#[test]
fn current_workspace_dirs_overview_preserves_publishable_hidden_entries_answer() {
    let state = test_state();
    let task = claimed_task("task-current-workspace-hidden-entries-preserve");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":3,"files":2,"hidden":5,"total":5},"entries":[{"hidden":true,"kind":"dir","name":".git","path":".git"},{"hidden":true,"kind":"file","name":".gitignore","path":".gitignore"},{"hidden":false,"kind":"file","name":"README.md","path":"README.md"}],"include_hidden":true,"names":[".git",".gitignore","README.md"],"path":"."}}"#,
    ));
    let answer = "有，例如：.gitignore、.codex、.git";
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        !replace_delivery_with_deterministic_current_workspace_dirs_overview_answer(
            &state,
            &task,
            "看一下当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
    assert!(summary.is_none());
}

#[test]
fn current_workspace_dirs_overview_does_not_replace_after_wrapped_read_range_evidence() {
    let state = test_state();
    let task = claimed_task("task-current-workspace-dirs-read-range");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":4,\"files\":1,\"total\":5},\"names_by_kind\":{\"dirs\":[\"UI\",\"configs\",\"crates\",\"docs\"],\"files\":[\"README.md\"],\"other\":[]},\"path\":\"workspace\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":4,"files":1,"total":5},"names_by_kind":{"dirs":["UI","configs","crates","docs"],"files":["README.md"],"other":[]},"path":"workspace"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"request_id":"req-2","status":"ok","text":"{\"action\":\"read_range\",\"path\":\"README.md\",\"excerpt\":\"1|# RustClaw\\n2|A local Rust agent runtime.\"}","error_text":null,"extra":{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|A local Rust agent runtime."}}"#,
    ));
    let synthesized = "RustClaw 是一个本地 Rust 智能体运行时。\n它围绕 `clawd` 组织多通道接入、任务执行和技能路由。\n工作区配置和 README 共同说明它面向可部署的日常控制台场景。";
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        synthesized,
    ));
    loop_state.delivery_messages.push(synthesized.to_string());
    loop_state.last_user_visible_respond = Some(synthesized.to_string());
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        !replace_delivery_with_deterministic_current_workspace_dirs_overview_answer(
            &state,
            &task,
            "把 RustClaw 当成当前项目来介绍，先查证项目 README 和工作区配置，再写三句话",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![synthesized.to_string()]);
    assert!(summary.is_none());
}

#[test]
fn current_workspace_dirs_overview_does_not_replace_freeform_project_article() {
    let state = test_state();
    let task = claimed_task("task-current-workspace-project-article");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":4,\"files\":1,\"total\":5},\"names_by_kind\":{\"dirs\":[\"UI\",\"configs\",\"crates\",\"docs\"],\"files\":[\"README.md\"],\"other\":[]},\"path\":\"workspace\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":4,"files":1,"total":5},"names_by_kind":{"dirs":["UI","configs","crates","docs"],"files":["README.md"],"other":[]},"path":"workspace"}}"#,
    ));
    let article = "RustClaw 是一个多渠道本地智能体运行时，本文将从架构、技能和部署展开。";
    loop_state.delivery_messages.push(article.to_string());
    loop_state.last_user_visible_respond = Some(article.to_string());
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        !replace_delivery_with_deterministic_current_workspace_dirs_overview_answer(
            &state,
            &task,
            "帮我写一篇关于 RustClaw 的长文",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![article.to_string()]);
    assert!(summary.is_none());
}

#[test]
fn recent_artifacts_judgment_replaces_non_answer_with_structured_inventory_verdict() {
    let task = claimed_task("task-recent-artifacts-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("Please run a directory listing so I can pick the newest entries.".to_string());
    loop_state.last_user_visible_respond =
        Some("Please run a directory listing so I can pick the newest entries.".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":3,"files":1,"hidden":0,"total":4},"entries":[{"hidden":false,"kind":"dir","modified_ts":1781284893,"name":"clarify_unpack_case","path":"scripts/nl_tests/fixtures/device_local/tmp/clarify_unpack_case","size_bytes":0},{"hidden":false,"kind":"dir","modified_ts":1781137621,"name":"manual_dynamic_guard_unpack","path":"scripts/nl_tests/fixtures/device_local/tmp/manual_dynamic_guard_unpack","size_bytes":0},{"hidden":false,"kind":"dir","modified_ts":1781135579,"name":"dynamic_guard_unpack_case","path":"scripts/nl_tests/fixtures/device_local/tmp/dynamic_guard_unpack_case","size_bytes":0},{"hidden":false,"kind":"file","modified_ts":1781095189,"name":"test_bundle.zip","path":"scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","size_bytes":1628}],"names":["clarify_unpack_case","manual_dynamic_guard_unpack","dynamic_guard_unpack_case","test_bundle.zip"],"path":"/repo/scripts/nl_tests/fixtures/device_local/tmp","resolved_path":"/repo/scripts/nl_tests/fixtures/device_local/tmp","size_summary":{"largest_file":{"hidden":false,"kind":"file","modified_ts":1781095189,"name":"test_bundle.zip","path":"scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","size_bytes":1628}},"sort_by":"mtime_desc"}}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "fs_basic",
        "read_file failed for /repo/scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip: stream did not contain valid UTF-8",
    ));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "scripts/nl_tests/fixtures/device_local/tmp".to_string();
    route.selection.list_selector.limit = Some(3);
    route.selection.list_selector.sort_by = Some("mtime_desc".to_string());
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("recent_entries.count=3"));
    assert!(answer.contains("recent_entries[0].name=clarify_unpack_case"));
    assert!(answer.contains("recent_entries[1].name=manual_dynamic_guard_unpack"));
    assert!(answer.contains("recent_entries[2].name=dynamic_guard_unpack_case"));
    assert!(!answer.contains("recent_entries[3].name=test_bundle.zip"));
    assert!(answer.contains("classification=temporary_bundle_artifact"));
    assert!(answer.contains("classification.formal_config=false"));
    assert!(answer.contains("classification.basis_tokens="));
    assert!(answer.contains("unpack"));
    assert!(summary.is_some());
}

#[test]
fn recent_artifacts_judgment_preserves_one_sentence_synthesis() {
    let task = claimed_task("task-recent-artifacts-preserve-synthesis");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1781284893,"name":"model_io.log","path":"logs/model_io.log","size_bytes":2300},{"hidden":false,"kind":"file","modified_ts":1781137621,"name":"nl_delayed_minimax_retry.log","path":"logs/nl_delayed_minimax_retry.log","size_bytes":60}],"names":["model_io.log","nl_delayed_minimax_retry.log"],"path":"/repo/logs","resolved_path":"/repo/logs","sort_by":"mtime_desc"}}"#,
    ));
    let synthesis = "最近最值得注意的是 MiniMax 延迟恢复日志，说明上游模型提供方曾触发等待重试。";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
    loop_state.delivery_messages.push(synthesis.to_string());
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::OneSentence;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![synthesis.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesis)
    );
    assert!(summary.is_none());
}

#[test]
fn recent_artifacts_judgment_replaces_count_only_machine_field_synthesis() {
    let task = claimed_task("task-recent-artifacts-count-only");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":2,"files":1,"hidden":0,"total":3},"entries":[{"hidden":false,"kind":"dir","modified_ts":1781635606,"name":"clarify_unpack_case","path":"scripts/nl_tests/fixtures/device_local/tmp/clarify_unpack_case","size_bytes":0},{"hidden":false,"kind":"file","modified_ts":1781480234,"name":"test_bundle.zip","path":"scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","size_bytes":272},{"hidden":false,"kind":"dir","modified_ts":1781137621,"name":"manual_dynamic_guard_unpack","path":"scripts/nl_tests/fixtures/device_local/tmp/manual_dynamic_guard_unpack","size_bytes":0}],"names":["clarify_unpack_case","test_bundle.zip","manual_dynamic_guard_unpack"],"path":"/repo/scripts/nl_tests/fixtures/device_local/tmp","resolved_path":"/repo/scripts/nl_tests/fixtures/device_local/tmp","sort_by":"mtime_desc"}}"#,
    ));
    let incomplete = "recent_entries.count=3";
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        incomplete,
    ));
    loop_state.last_publishable_synthesis_output = Some(incomplete.to_string());
    loop_state.last_user_visible_respond = Some(incomplete.to_string());
    loop_state.delivery_messages.push(incomplete.to_string());
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::OneSentence;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "scripts/nl_tests/fixtures/device_local/tmp".to_string();
    route.selection.list_selector.limit = Some(3);
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("recent_entries.count=3"));
    assert!(answer.contains("recent_entries[0].name=clarify_unpack_case"));
    assert!(answer.contains("recent_entries[1].name=test_bundle.zip"));
    assert!(answer.contains("recent_entries[2].name=manual_dynamic_guard_unpack"));
    assert!(answer.contains("classification.output_format=per_entry"));
    assert!(summary.is_some());
}

#[test]
fn recent_artifacts_judgment_detects_full_machine_field_delivery() {
    let delivery = [
        "recent_entries.count=3",
        "recent_entries.root=/repo/configs",
        "recent_entries.sort_by=mtime_desc",
        "recent_entries[0].name=rss.toml",
        "recent_entries[0].kind=file",
        "recent_entries[0].path=configs/rss.toml",
        "recent_entries[0].classification=formal_config",
        "classification.output_format=per_entry",
        "classification=formal_config",
        "classification.formal_config=true",
    ]
    .join("\n");

    assert!(directory_purpose::recent_artifacts_delivery_is_machine_field_dump(&delivery));
    assert!(
        !directory_purpose::recent_artifacts_delivery_is_machine_field_dump(
            "rss.toml, task_contract_matrix.toml, config.toml look like runtime config."
        )
    );
}

#[test]
fn recent_artifacts_judgment_respects_file_selector_before_limit() {
    let task = claimed_task("task-recent-artifacts-file-selector");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","entries":[{"kind":"file","modified_ts":9,"name":"config.toml","path":"configs/config.toml","size_bytes":23851},{"kind":"file","modified_ts":8,"name":"task_contract_matrix.toml","path":"configs/task_contract_matrix.toml","size_bytes":28845},{"kind":"dir","modified_ts":7,"name":"i18n","path":"configs/i18n","size_bytes":0},{"kind":"file","modified_ts":6,"name":"agent_guard.toml","path":"configs/agent_guard.toml","size_bytes":17952}],"path":"/repo/configs","sort_by":"mtime_desc"}"#,
    ));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    route.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.selection.list_selector.target_kind_specified = true;
    route.selection.list_selector.limit = Some(3);
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("recent_entries.count=3"));
    assert!(answer.contains("recent_entries[0].name=config.toml"));
    assert!(answer.contains("recent_entries[0].classification=formal_config"));
    assert!(answer.contains("recent_entries[1].name=task_contract_matrix.toml"));
    assert!(answer.contains("recent_entries[1].classification=formal_config"));
    assert!(answer.contains("recent_entries[2].name=agent_guard.toml"));
    assert!(answer.contains("recent_entries[2].classification=formal_config"));
    assert!(!answer.contains("recent_entries[2].name=i18n"));
    assert!(!answer.contains("recent_entries[3]"));
    assert!(answer.contains("classification.output_format=per_entry"));
    assert!(answer.contains("classification=formal_config"));
    assert!(summary.is_some());
}

#[test]
fn recent_artifacts_judgment_classifies_scripts_per_entry() {
    let task = claimed_task("task-recent-artifacts-scripts-per-entry");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","entries":[{"kind":"file","modified_ts":9,"name":"summarize_agent_decides_route_delta.py","path":"scripts/summarize_agent_decides_route_delta.py","size_bytes":42200},{"kind":"file","modified_ts":8,"name":"check_long_files.py","path":"scripts/check_long_files.py","size_bytes":5839},{"kind":"file","modified_ts":7,"name":"sync_registry_governance_fields.py","path":"scripts/sync_registry_governance_fields.py","size_bytes":3531}],"path":"/repo/scripts","sort_by":"mtime_desc"}"#,
    ));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    route.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.selection.list_selector.target_kind_specified = true;
    route.selection.list_selector.limit = Some(3);
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("recent_entries.count=3"));
    assert!(answer.contains("classification.output_format=per_entry"));
    assert!(answer.contains("recent_entries[0].classification=maintenance_script"));
    assert!(answer.contains("recent_entries[1].classification=maintenance_script"));
    assert!(answer.contains("recent_entries[2].classification=maintenance_script"));
    assert!(answer.contains("recent_entries[0].business_data=false"));
    assert!(answer.contains("recent_entries[1].business_data=false"));
    assert!(answer.contains("recent_entries[2].business_data=false"));
    assert!(answer.contains("classification=maintenance_script"));
    assert!(!answer.contains("classification=runtime_log"));
    assert!(summary.is_some());
}

#[test]
fn recent_artifacts_judgment_classifies_logs_per_entry() {
    let task = claimed_task("task-recent-artifacts-logs-per-entry");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","entries":[{"kind":"file","modified_ts":9,"name":"clawd.run.log","path":"logs/clawd.run.log","size_bytes":2300},{"kind":"file","modified_ts":8,"name":"model_io.log","path":"logs/model_io.log","size_bytes":900}],"names":["clawd.run.log","model_io.log"],"path":"/repo/logs","resolved_path":"/repo/logs","sort_by":"mtime_desc"}}"#,
    ));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    route.selection.list_selector.limit = Some(2);
    route.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.selection.list_selector.target_kind_specified = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("recent_entries.count=2"));
    assert!(answer.contains("recent_entries[0].name=clawd.run.log"));
    assert!(answer.contains("recent_entries[0].classification=runtime_log"));
    assert!(answer.contains("recent_entries[1].name=model_io.log"));
    assert!(answer.contains("recent_entries[1].classification=runtime_log"));
    assert!(answer.contains("classification=runtime_log"));
    assert!(answer.contains("classification.business_data=false"));
    assert!(summary.is_some());
}

#[test]
fn recent_artifacts_judgment_classifies_docs_markdown_per_entry() {
    let task = claimed_task("task-recent-artifacts-docs-per-entry");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","entries":[{"kind":"file","modified_ts":9,"name":"planning_full_test_failure_inventory.md","path":"docs/planning_full_test_failure_inventory.md","size_bytes":9320},{"kind":"file","modified_ts":8,"name":"long_file_split_inventory.md","path":"docs/long_file_split_inventory.md","size_bytes":13375},{"kind":"file","modified_ts":7,"name":"legacy_semantic_route_inventory.md","path":"docs/legacy_semantic_route_inventory.md","size_bytes":6072}],"path":"/repo/docs","sort_by":"mtime_desc"}"#,
    ));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    route.selection.list_selector.limit = Some(3);
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("classification.output_format=per_entry"));
    assert!(answer.contains("recent_entries[0].classification=project_documentation"));
    assert!(answer.contains("recent_entries[1].classification=project_documentation"));
    assert!(answer.contains("recent_entries[2].classification=project_documentation"));
    assert!(answer.contains("classification=project_documentation"));
    assert!(!answer.contains("classification=unknown"));
    assert!(summary.is_some());
}

#[test]
fn recent_artifacts_judgment_replaces_unqualified_delivery_even_when_entries_are_mentioned() {
    let task = claimed_task("task-recent-artifacts-preserve");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    let delivery =
        "clarify_unpack_case, manual_dynamic_guard_unpack, dynamic_guard_unpack_case are recent.";
    loop_state.delivery_messages.push(delivery.to_string());
    loop_state.last_user_visible_respond = Some(delivery.to_string());
    loop_state.last_publishable_synthesis_output = Some(delivery.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","entries":[{"kind":"dir","modified_ts":3,"name":"clarify_unpack_case","path":"tmp/clarify_unpack_case"},{"kind":"dir","modified_ts":2,"name":"manual_dynamic_guard_unpack","path":"tmp/manual_dynamic_guard_unpack"},{"kind":"dir","modified_ts":1,"name":"dynamic_guard_unpack_case","path":"tmp/dynamic_guard_unpack_case"}],"path":"tmp","sort_by":"mtime_desc"}"#,
    ));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            &task,
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("recent_entries.count=3"));
    assert!(answer.contains("classification=temporary_bundle_artifact"));
    assert!(summary.is_some());
}
