// Parse-failed and fallback repair tests for intent_router.

use crate::{execution_recipe::ExecutionRecipeKind, FirstLayerDecision};

use super::test_support::make_temp_workspace_with_child;
use super::{
    normalizer_output_from_fallback, parse_execution_recipe_hint, IntentExecutionRecipeOut,
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, RouteDecision, ScheduleKind, TargetTaskPolicy, TurnType,
};

fn test_task(task_id: &str, text: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({ "text": text }).to_string(),
    }
}

#[test]
fn current_turn_locator_sanitizer_drops_contextual_path_prefix() {
    let req = "读一下 README.md 然后用恰好三句话总结，不要多也不要少";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let cleaned = super::sanitize_resolved_intent_for_current_turn_locator(
        "读取 docs/README.md 文件内容并用恰好三句话进行总结",
        req,
        &surface,
    );

    assert_eq!(cleaned.as_deref(), Some(req));
}

#[test]
fn current_turn_locator_sanitizer_ignores_bare_stem_without_extension() {
    let req = "读一下 README 然后用恰好三句话总结，不要多也不要少";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let cleaned = super::sanitize_resolved_intent_for_current_turn_locator(
        "读取 document 目录下的 README.md 文件内容并用恰好三句话进行总结",
        req,
        &surface,
    );

    assert_eq!(cleaned, None);
}

#[test]
fn normalizer_schema_normalization_does_not_invent_contract_from_surface() {
    let raw = r#"{
          "resolved_user_intent": "检查当前目录是否有隐藏文件，如有则列出3个例子",
          "needs_clarify": false,
          "reason": "local hidden entries check",
          "confidence": 0.98,
          "decision":"planner_execute"
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("free")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_extracts_fenced_json() {
    let raw = r#"```json
{
  "resolved_user_intent": "检查当前目录有没有隐藏文件，只回答有或没有，并补3个例子",
  "needs_clarify": false,
  "reason": "local hidden entries check",
  "confidence": 0.95,
  "decision":"planner_execute",
  "output_contract": {
    "response_shape": "scalar",
    "requires_content_evidence": true,
    "semantic_kind": "hidden_files_example",
    "locator_kind": "current_workspace"
  }
}
```"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("hidden_entries_check")
    );
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("scalar")
    );
    assert_eq!(
        contract.get("locator_kind").and_then(|v| v.as_str()),
        Some("current_workspace")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_preserves_act_when_shape_is_descriptive() {
    let raw = r#"{
          "resolved_user_intent": "列出 logs 目录下前 10 个文件名，不读取内容",
          "needs_clarify": false,
          "reason": "workspace directory listing",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": {
            "response_shape": "list_of_strings",
            "semantic_kind": "file_names"
          },
          "action": {"tool":"list_directory","path":"logs","limit":10}
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "列出 logs 目录下的前 10 个文件名");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value.get("needs_clarify").and_then(|v| v.as_bool()),
        Some(false)
    );
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("strict")
    );
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("file_names")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_coerces_output_contract_scalar_and_aliases() {
    let raw = r#"{
          "resolved_user_intent": "列出 README.md 和 AGENTS.md，只输出文件名",
          "needs_clarify": false,
          "reason": "names-only inventory",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": "file_names"
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "列出文件，只输出文件名");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("none")
    );

    let raw = r#"{
          "resolved_user_intent": "严格输出两行",
          "needs_clarify": false,
          "reason": "exact output",
          "confidence": 0.9,
          "decision":"direct_answer",
          "output_contract": {"shape":"exact_format","semantic":"sqlite_table_names"}
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "严格输出两行");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("strict")
    );
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        contract
            .get("requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_maps_file_type_filename_to_locator_hint() {
    let raw = r#"{
          "resolved_user_intent": "User wants to receive the file from the current workspace.",
          "needs_clarify": false,
          "reason": "file delivery",
          "confidence": 0.95,
          "decision": "planner_execute",
          "wants_file_delivery": true,
          "output_contract": {
            "type": "file",
            "filename": "definitely_missing_named_file_phase0_runtime_20260515.txt"
          }
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "把 definitely_missing_named_file_phase0_runtime_20260515.txt 发给我",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("file_token")
    );
    assert_eq!(
        contract.get("delivery_required").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        contract.get("locator_kind").and_then(|v| v.as_str()),
        Some("filename")
    );
    assert_eq!(
        contract.get("locator_hint").and_then(|v| v.as_str()),
        Some("definitely_missing_named_file_phase0_runtime_20260515.txt")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn safe_fallback_tries_llm_except_when_model_unavailable() {
    assert!(!super::safe_fallback_source_should_try_llm(
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    ));
    assert!(super::safe_fallback_source_should_try_llm(
        crate::fallback::ClarifyFallbackSource::IntentUnresolved
    ));
    assert!(super::safe_fallback_source_should_try_llm(
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty
    ));
}

#[test]
fn parse_execution_recipe_hint_missing_profile_falls_back_to_default_spec() {
    // 历史语义：profile 缺失 → None（曾让下游 fallback 到 keyword detect）
    // B1 修复后：normalizer 显式回了 execution_recipe 字段（即使 profile 缺）就视为
    // 已分类，返回 default spec（kind=None, inactive），不再触发本地补判。
    // 这样可以避免 legacy local detector 因 STABLE_FACTS 污染而误升级 read-only 任务。
    let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
        kind: "ops_closed_loop".to_string(),
        profile: String::new(),
        target_scope: "current_repo".to_string(),
        ..IntentExecutionRecipeOut::default()
    }))
    .expect("normalizer-classified hint should yield Some, even with missing profile");
    assert_eq!(spec.kind, ExecutionRecipeKind::None);
}

#[test]
fn parse_execution_recipe_hint_explicit_none_is_trusted() {
    // 这是修复 B1 的核心回归测试。
    // 场景：normalizer 已经基于完整上下文判定"这不是 ops loop"（kind=none）。
    // 期望：返回 Some(default spec) → initial_execution_recipe_spec 用 default spec
    // → runtime.is_active()=false → plan_repair_reason 不会触发
    // ops_closed_loop_apply_requires_mutation。
    // 历史风险：返回 None 会让下游 fallback 到 keyword 启发式；
    // 长期记忆里残留的 "configs/" "verify" 关键字会把任务误升级为
    // OpsClosedLoop config_change，让 read-only 的 `pwd` 任务跑挂。
    let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
        kind: "none".to_string(),
        profile: "none".to_string(),
        target_scope: "unknown".to_string(),
        ..IntentExecutionRecipeOut::default()
    }))
    .expect("explicit kind=none should still be Some so local fallback remains bypassed");
    assert_eq!(spec.kind, ExecutionRecipeKind::None);
    assert!(
        !crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(spec).is_active(),
        "default spec must produce an inactive runtime state"
    );
}

#[test]
fn parse_execution_recipe_hint_missing_field_leaves_no_recipe_hint() {
    // 当 normalizer 完全没在响应里给出 execution_recipe 字段时（None），
    // 只表示 LLM 没给出 recipe hint；主链不再用本地关键词检测补判。
    assert!(parse_execution_recipe_hint(None).is_none());
}

#[test]
fn fallback_normalizer_output_still_enforces_content_evidence_planner_execute() {
    let out = normalizer_output_from_fallback(
        "把当前目录有没有隐藏文件看一下",
        "parse_failed_fallback_router",
        RouteDecision {
            resolved_user_intent: "看一下当前目录有没有隐藏文件".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "current workspace executable request".to_string(),
            confidence: Some(0.72),
            schedule_kind: super::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        },
        None,
    );
    assert_eq!(out.route_trace_decision, FirstLayerDecision::PlannerExecute);
    assert!(!out.needs_clarify);
    assert_eq!(
        out.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(out.fallback_source, None);
}

#[test]
fn parse_failed_fallback_no_longer_builds_git_semantic_contract() {
    let req = "只告诉我当前 git 分支名。";
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = test_task("parse-failed-no-git-semantic-contract", req);
    let fallback = super::normalizer_parse_failed_fallback_output(
        &state,
        &task,
        req,
        req,
        &crate::intent::surface_signals::analyze_prompt_surface(req),
        "{not-json",
    );

    assert_eq!(fallback.route_trace_decision, FirstLayerDecision::Clarify);
    assert!(fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::None
    );
    assert!(!fallback.output_contract.requires_content_evidence);
}

#[test]
fn parse_failed_existing_directory_path_fallback_builds_observation_contract() {
    let root = make_temp_workspace_with_child("parse_failed_existing_dir", "docs");
    std::fs::write(
        root.join("docs").join("release_checklist.md"),
        "# Release Checklist",
    )
    .expect("write fixture doc");
    let req = "先列出 docs/ 目录里的文件名，再读取 release_checklist.md 开头，最后用一句中文判断它是什么类型";

    let fallback =
        super::parse_failed_explicit_existing_path_observation_fallback_decision(req, &root)
            .expect("existing directory fallback");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::DirectoryPurposeSummary
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::Path
    );
    assert!(fallback.output_contract.requires_content_evidence);
    assert!(fallback.output_contract.locator_hint.ends_with("/docs"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn parse_failed_existing_file_path_fallback_builds_content_contract() {
    let root = make_temp_workspace_with_child("parse_failed_existing_file", "docs");
    std::fs::write(
        root.join("docs").join("release_checklist.md"),
        "# Release Checklist",
    )
    .expect("write fixture doc");
    let req = "读取 docs/release_checklist.md 开头并用一句话总结";

    let fallback =
        super::parse_failed_explicit_existing_path_observation_fallback_decision(req, &root)
            .expect("existing file fallback");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::Path
    );
    assert!(fallback.output_contract.requires_content_evidence);
    assert!(fallback
        .output_contract
        .locator_hint
        .ends_with("/docs/release_checklist.md"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn parse_failed_archive_unpack_pair_fallback_preserves_archive_contract() {
    let root = make_temp_workspace_with_child("parse_failed_archive_unpack_pair", "tmp");
    let archive = root.join("tmp").join("test_bundle.zip");
    std::fs::write(&archive, b"zip-placeholder").expect("write archive placeholder");
    let dest = root.join("tmp").join("manual_dynamic_guard_unpack");
    let req = format!(
        "extract {} into {} and report the result",
        archive.display(),
        dest.display()
    );

    let fallback =
        super::parse_failed_explicit_existing_path_observation_fallback_decision(&req, &root)
            .expect("archive pair fallback");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::ArchiveUnpack
    );
    assert_eq!(
        fallback.output_contract.response_shape,
        OutputResponseShape::OneSentence
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::Path
    );
    assert_eq!(
        fallback.output_contract.locator_hint,
        format!("{} | {}", archive.display(), dest.display())
    );
    assert!(fallback.output_contract.requires_content_evidence);
    assert!(!fallback.output_contract.delivery_required);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inline_json_transform_fallback_builds_planner_execute_contract() {
    let req = r#"{"action":"transform_data","data":[{"team":"A","score":5},{"team":"A","score":7},{"team":"B","score":3}],"ops":[{"op":"group","by":["team"],"aggregations":[{"op":"sum","field":"score","name":"total"}]}]}"#;
    let fallback = super::inline_json_transform_fallback_decision(req)
        .expect("structured inline transform fallback");
    let out = normalizer_output_from_fallback(
        req,
        "llm_failed_inline_json_transform_fallback",
        fallback,
        None,
    );

    assert_eq!(out.route_trace_decision, FirstLayerDecision::PlannerExecute);
    assert!(!out.needs_clarify);
    assert_eq!(
        out.output_contract.response_shape,
        OutputResponseShape::Strict
    );
    assert!(out.output_contract.requires_content_evidence);
    assert_eq!(out.output_contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(out.output_contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(out.fallback_source, None);
}

#[test]
fn inline_json_transform_fallback_ignores_non_structured_text() {
    let req = "please transform the score data someday";

    assert!(super::inline_json_transform_fallback_decision(req).is_none());
}

#[test]
fn parsed_inline_json_transform_repair_builds_planner_execute_contract() {
    let req = r#"把这个 JSON 对象里的 old_name 改成 new_name，只输出 JSON：{"old_name":"alpha","count":2}"#;
    let fallback = super::parsed_inline_json_transform_repair_decision(
        req,
        true,
        false,
        ScheduleKind::None,
        None,
    )
    .expect("parsed inline transform repair");

    assert_eq!(
        fallback.reason,
        "parsed_inline_json_transform_contract_repair"
    );
    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.response_shape,
        OutputResponseShape::Strict
    );
    assert!(fallback.output_contract.requires_content_evidence);
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::None
    );
}

#[test]
fn parsed_inline_json_transform_repair_preserves_file_delivery_route() {
    let req = r#"sort this JSON array by score descending: [{"name":"alpha","score":7}]"#;

    assert!(super::parsed_inline_json_transform_repair_decision(
        req,
        true,
        true,
        ScheduleKind::None,
        None
    )
    .is_none());
}

#[test]
fn parsed_inline_json_transform_repair_preserves_clean_planner_route() {
    let req = r#"计算这个 JSON 数组里 value 的总和，只输出数字：[{"value":4},{"value":6}]"#;

    assert!(super::parsed_inline_json_transform_repair_decision(
        req,
        false,
        false,
        ScheduleKind::None,
        None
    )
    .is_none());
}

#[test]
fn parsed_inline_json_transform_repair_preserves_direct_non_clarify_route() {
    let req = r#"计算这个 JSON 数组里 value 的总和，只输出数字：[{"value":4},{"value":6}]"#;

    assert!(super::parsed_inline_json_transform_repair_decision(
        req,
        false,
        false,
        ScheduleKind::None,
        None
    )
    .is_none());
}

#[test]
fn directory_pair_fallback_builds_planner_execute_locator_contract() {
    let root = make_temp_workspace_with_child("directory_pair_fallback", "seed");
    std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case")).expect("right");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let req = "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.";
    let fallback = super::directory_pair_fallback_decision(&state, req)
        .expect("resolved directory pair fallback");
    let out =
        normalizer_output_from_fallback(req, "llm_failed_directory_pair_fallback", fallback, None);

    assert_eq!(out.route_trace_decision, FirstLayerDecision::PlannerExecute);
    assert!(!out.needs_clarify);
    assert_eq!(
        out.output_contract.response_shape,
        OutputResponseShape::Strict
    );
    assert!(out.output_contract.requires_content_evidence);
    assert_eq!(out.output_contract.locator_kind, OutputLocatorKind::Path);
    assert!(out.output_contract.locator_hint.contains("bundle_src"));
    assert!(out
        .output_contract
        .locator_hint
        .contains("dynamic_guard_unpack_case"));
    assert_eq!(out.output_contract.semantic_kind, OutputSemanticKind::None);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_fallback_ignores_unresolved_text() {
    let state = crate::AppState::test_default_with_fixture_provider();

    assert!(super::directory_pair_fallback_decision(
        &state,
        "compare source_alpha and source_beta"
    )
    .is_none());
}

#[test]
fn explicit_surface_path_facts_fallback_builds_existence_contract() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let fallback = super::explicit_surface_path_facts_fallback_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
    )
    .expect("explicit multi-path facts fallback");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert!(fallback.output_contract.requires_content_evidence);
}

#[test]
fn explicit_surface_path_facts_clarify_repair_overrides_missing_path_clarify() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let fallback = super::explicit_surface_path_facts_clarify_repair_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
        true,
        &IntentOutputContract::default(),
        false,
        ScheduleKind::None,
        None,
    )
    .expect("explicit multi-path clarify repair");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(fallback.reason, "boundary_explicit_multi_path_facts");
}

#[test]
fn explicit_surface_path_metadata_clarify_repair_preserves_quantity_comparison() {
    let req = "Inspect metadata for scripts/nl_tests/fixtures/device_local/package.json and scripts/nl_tests/fixtures/device_local/configs/app_config.toml.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::QuantityComparison,
        ..Default::default()
    };
    let fallback = super::explicit_surface_path_metadata_clarify_repair_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
        true,
        &contract,
        false,
        ScheduleKind::None,
        None,
    )
    .expect("explicit metadata clarify repair");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::QuantityComparison
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(fallback.reason, "boundary_explicit_multi_path_metadata");
}

#[test]
fn explicit_surface_path_facts_clarify_repair_preserves_structured_contract() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..Default::default()
    };

    assert!(super::explicit_surface_path_facts_clarify_repair_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
        true,
        &contract,
        false,
        ScheduleKind::None,
        None,
    )
    .is_none());
}

#[test]
fn explicit_surface_path_facts_fallback_ignores_single_path() {
    let req = "scripts/nl_tests/fixtures/device_local/package.json";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);

    assert!(super::explicit_surface_path_facts_fallback_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
    )
    .is_none());
}

#[test]
fn workspace_scope_patch_sets_locator_hint_from_structured_scope() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        locator_hint: "/home/guagua/rustclaw".to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let applied = super::apply_workspace_scope_patch_to_contract(
        "workspace_project_summary",
        &mut contract,
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        Some(&serde_json::json!({"scope": "UI_only"})),
    );

    assert_eq!(applied.as_deref(), Some("UI"));
    assert_eq!(contract.locator_hint, "UI");
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn workspace_scope_patch_keeps_specific_locator_hint() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        locator_hint: "UI".to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let applied = super::apply_workspace_scope_patch_to_contract(
        "workspace_project_summary",
        &mut contract,
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        Some(&serde_json::json!({"scope": "pi_app_only"})),
    );

    assert_eq!(applied, None);
    assert_eq!(contract.locator_hint, "UI");
}

#[test]
fn fallback_normalizer_keeps_llm_failure_on_safe_clarify() {
    let out = normalizer_output_from_fallback(
        "read scripts/nl_tests/fixtures/device_local/package.json and output only the name field",
        "llm_failed_safe_clarify",
        RouteDecision {
            resolved_user_intent: String::new(),
            needs_clarify: true,
            clarify_question: String::new(),
            reason: "fallback_router_llm_failed".to_string(),
            confidence: None,
            schedule_kind: super::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract::default(),
        },
        Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
    );
    assert_eq!(out.route_trace_decision, FirstLayerDecision::Clarify);
    assert!(out.needs_clarify);
    assert!(matches!(
        out.output_contract.response_shape,
        OutputResponseShape::Free
    ));
    assert!(!out.output_contract.requires_content_evidence);
    assert!(!out.output_contract.delivery_required);
    assert!(matches!(
        out.output_contract.locator_kind,
        OutputLocatorKind::None
    ));
    assert!(matches!(
        out.output_contract.delivery_intent,
        OutputDeliveryIntent::None
    ));
    assert!(out
        .reason
        .contains("llm_failed_safe_clarify; fallback_router_llm_failed"));
    assert_eq!(
        out.fallback_source,
        Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable)
    );
}
