use super::*;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_ask_prepare_{name}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct TempFileGuard {
    path: std::path::PathBuf,
}

impl TempFileGuard {
    fn new(name: &str, contents: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_ask_prepare_{name}_{}_{}.toml",
            std::process::id(),
            nanos
        ));
        fs::write(&path, contents).expect("write temp file");
        Self { path }
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn test_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "ask-prepare-structured-field-test".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("ask-prepare-structured-field-user".to_string()),
        channel: "api".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[test]
fn active_clarify_locator_fast_path_preserves_structured_field_selector_token() {
    let root = TempDirGuard::new("clarify_structured_field_selector");
    fs::write(root.path.join("package.json"), r#"{"name":"rustclaw"}"#)
        .expect("write package json");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let task = test_task();
    let source_request = "读一下那个文件里的名字字段，只输出值 structured_field_selector=name";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供要读取的文件路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(
                crate::OutputSemanticKind::StructuredKeys
                    .as_str()
                    .to_string(),
            ),
            source_request: source_request.to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution = active_clarify_existing_workspace_locator_reply(
        &root.path,
        &root.path,
        "package.json",
        &snapshot,
    )
    .expect("existing package path should resolve");

    let route = active_clarify_locator_reply_fast_path_route(&state, &task, &snapshot, &resolution)
        .expect("active clarify scalar locator reply should use fast path");

    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("name")
    );
    assert!(route
        .route_reason
        .contains("preserve_active_clarify_structured_field_selector"));
}

#[test]
fn active_clarify_package_json_locator_strips_package_selector_prefix() {
    let root = TempDirGuard::new("clarify_package_json_selector");
    fs::write(root.path.join("package.json"), r#"{"name":"rustclaw"}"#)
        .expect("write package json");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let task = test_task();
    let source_request = "Read the package name field only structured_field_selector=package.name";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Please provide the file path.".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(
                crate::OutputSemanticKind::StructuredKeys
                    .as_str()
                    .to_string(),
            ),
            source_request: source_request.to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution = active_clarify_existing_workspace_locator_reply(
        &root.path,
        &root.path,
        "package.json",
        &snapshot,
    )
    .expect("existing package path should resolve");

    let route = active_clarify_locator_reply_fast_path_route(&state, &task, &snapshot, &resolution)
        .expect("active clarify scalar locator reply should use fast path");

    assert!(route.is_execute_gate());
    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("name")
    );
    assert!(route
        .route_reason
        .contains("normalize_active_clarify_structured_field_selector"));
}

#[test]
fn scalar_field_selector_repairs_document_heading_contract_to_field_value_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read a structured file field value".to_string(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::DocumentHeading,
            locator_hint:
                "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/package.json"
                    .to_string(),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("description".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };

    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "Read /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/package.json and extract the selected structured field value",
    );

    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_field_value_contract_repair"));
}

#[test]
fn single_locator_field_selector_does_not_bind_to_scalar_pair_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read one structured field value".to_string(),
        needs_clarify: false,
        route_reason: "structured_field_selector_requires_scalar_value".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "UI/package.json".to_string(),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("scripts.dev".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };

    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "读取 UI/package.json 里的 scripts.dev，只输出值。",
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(!route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
}

#[test]
fn single_path_field_selector_repairs_misclassified_equality_contract_to_scalar_value() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read the selected TOML field value only".to_string(),
        needs_clarify: false,
        route_reason:
            "structured_field_selector_requires_scalar_value; recent_scalar_equality_check"
                .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint: "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
                .to_string(),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("paths.db_path".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };

    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "Output only paths.db_path from scripts/nl_tests/fixtures/device_local/configs/app_config.toml.",
    );

    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_field_value_contract_repair"));
    assert!(!route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
}

#[test]
fn scalar_field_selector_repairs_parent_selector_to_existing_leaf_value() {
    let target = TempFileGuard::new(
        "workspace_dependencies",
        "[workspace]\n[workspace.dependencies]\ntoml = \"0.8\"\n",
    );
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read workspace.dependencies.toml from Cargo.toml".to_string(),
        needs_clarify: false,
        route_reason: "structured_field_selector_requires_scalar_value".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::StructuredKeys,
            locator_hint: target.path.display().to_string(),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("workspace.dependencies".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };

    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "Read workspace.dependencies.toml from ./Cargo.toml and output only the value.",
    );

    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("workspace.dependencies.toml")
    );
    assert!(route
        .route_reason
        .contains("structured_field_selector_exact_target_repair"));
}

#[test]
fn structured_field_target_repair_moves_dotted_field_locator_to_structured_file() {
    let root = TempDirGuard::new("workspace_dependencies_field_locator");
    fs::write(
        root.path.join("Cargo.toml"),
        "[workspace]\n[workspace.dependencies]\ntoml = \"0.8\"\n",
    )
    .expect("write cargo toml");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "Read workspace.dependencies.toml from ./Cargo.toml and output only the value."
                .to_string(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Filename,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: "workspace.dependencies.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    repair_structured_field_target_from_prompt(
        &mut route,
        "Read workspace.dependencies.toml from ./Cargo.toml and output only the value.",
        &root.path,
        &root.path,
    );
    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "Read workspace.dependencies.toml from ./Cargo.toml and output only the value.",
    );

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.path.join("Cargo.toml").display().to_string()
    );
    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("workspace.dependencies.toml")
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .route_reason
        .contains("structured_field_target_from_prompt_repair"));
}

#[test]
fn structured_field_with_text_target_refines_to_recent_scalar_contract() {
    let root = TempDirGuard::new("structured_field_text_pair");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.8"
"#,
    )
    .expect("write cargo manifest");
    fs::write(root.path.join("README.md"), "version: 0.1.8\n").expect("write readme");

    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "Read workspace.package.version from Cargo.toml and compare it with README.md"
                .to_string(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: root.path.display().to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let prompt = "Read workspace.package.version from Cargo.toml and compare it with the version mentioned in README.md, then answer in one sentence.";

    repair_structured_field_target_from_prompt(&mut route, prompt, &root.path, &root.path);
    assert!(route
        .route_reason
        .contains("structured_field_target_from_prompt_repair"));
    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("workspace.package.version")
    );

    repair_scalar_field_value_contract_for_locator_reply(&mut route, prompt);

    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::RecentScalarEqualityCheck
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
    assert!(route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
}

#[test]
fn command_summary_structured_field_target_repair_preserves_summary_contract() {
    let root = TempDirGuard::new("structured_field_command_summary_preserve");
    let readme = root.path.join("README.md");
    let config = root.path.join("config.toml");
    fs::write(&readme, "# RustClaw\n").expect("write readme");
    fs::write(&config, "[llm]\nselected_vendor = \"minimax\"\n").expect("write config");

    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "Summarize local observations with README existence, config field, cwd, and clock."
                .to_string(),
        needs_clarify: false,
        route_reason: "command_output_summary".to_string(),
        route_confidence: Some(0.93),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::CommandOutputSummary,
            locator_hint: root.path.display().to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let prompt = format!(
        "Check whether {} exists; read llm.selected_vendor from {}; also include cwd and current time in one table.",
        readme.display(),
        config.display()
    );

    repair_structured_field_target_from_prompt(&mut route, &prompt, &root.path, &root.path);
    repair_scalar_field_value_contract_for_locator_reply(&mut route, &prompt);

    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("llm.selected_vendor")
    );
    assert!(route
        .route_reason
        .contains("multi_locator_structured_field_preserves_summary_contract"));
    assert!(!route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
}

#[test]
fn multi_locator_structured_field_preserves_summary_contract() {
    let root = TempDirGuard::new("structured_field_summary_bundle");
    let readme = root.path.join("README.md");
    let config = root.path.join("config.toml");
    fs::write(&readme, "# RustClaw\n").expect("write readme");
    fs::write(&config, "[llm]\nselected_vendor = \"minimax\"\n").expect("write config");

    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "Combine file existence, config field, working directory, and clock observations."
                .to_string(),
        needs_clarify: false,
        route_reason: concat!(
            "compound_local_observation_summary; command_output_summary; ",
            "command_result_synthesis; structured_field_selector=llm.selected_vendor; ",
            "structured_field_target_from_prompt_repair"
        )
        .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: config.display().to_string(),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("llm.selected_vendor".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };
    let prompt = format!(
        "Check whether {} exists; read llm.selected_vendor from {}; also include cwd and current time in one table.",
        readme.display(),
        config.display()
    );

    repair_scalar_field_value_contract_for_locator_reply(&mut route, &prompt);

    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
    assert!(route
        .route_reason
        .contains("multi_locator_structured_field_preserves_summary_contract"));
    assert!(!route
        .route_reason
        .contains("scalar_field_value_contract_repair"));
    assert!(!route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
}

#[test]
fn multi_locator_summary_contract_requires_exact_summary_marker() {
    let root = TempDirGuard::new("structured_field_summary_exact_marker");
    let readme = root.path.join("README.md");
    let config = root.path.join("config.toml");
    fs::write(&readme, "# RustClaw\n").expect("write readme");
    fs::write(&config, "[llm]\nselected_vendor = \"minimax\"\n").expect("write config");

    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Combine file existence and config field observations.".to_string(),
        needs_clarify: false,
        route_reason: concat!(
            "compound local observation summary command_output_summary_extra ",
            "command_result_synthesis_extra; structured_field_target_from_prompt_repair"
        )
        .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: config.display().to_string(),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("llm.selected_vendor".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };
    let prompt = format!(
        "Check whether {} exists; read llm.selected_vendor from {}.",
        readme.display(),
        config.display()
    );

    repair_scalar_field_value_contract_for_locator_reply(&mut route, &prompt);

    assert_ne!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::CommandOutputSummary
    );
    assert!(route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
}

#[test]
fn three_target_structured_field_request_preserves_summary_contract() {
    let root = TempDirGuard::new("structured_field_three_target_summary");
    let readme = root.path.join("README.md");
    let docs = root.path.join("docs");
    let config = root.path.join("config.toml");
    fs::write(&readme, "# RustClaw\n").expect("write readme");
    fs::create_dir_all(&docs).expect("create docs");
    fs::write(docs.join("service_notes.md"), "status ok\n").expect("write docs");
    fs::write(&config, "[skills.fs_basic]\nplanner_kind = \"builtin\"\n").expect("write config");

    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "Collect file existence, directory listing, and one config field into a table."
                .to_string(),
        needs_clarify: false,
        route_reason: "structured_field_selector_requires_scalar_value".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: format!(
                "{}; {}; {}",
                readme.display(),
                docs.display(),
                config.display()
            ),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("skills.fs_basic.planner_kind".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };
    let prompt = format!(
        "Check whether {} exists; list filenames under {}; read skills.fs_basic.planner_kind from {}; return a table.",
        readme.display(),
        docs.display(),
        config.display()
    );

    repair_scalar_field_value_contract_for_locator_reply(&mut route, &prompt);

    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::CommandOutputSummary
    );
    assert!(route
        .route_reason
        .contains("multi_locator_structured_field_preserves_summary_contract"));
    assert!(!route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
}
