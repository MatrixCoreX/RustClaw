use super::super::background_only_locator_route_should_force_clarify;
use super::prebind_quantity_compare_directory_pair_from_current_request;
use crate::{AgentRuntimeConfig, AppState, SkillViewsSnapshot};
use claw_core::config::{AgentConfig, ToolsConfig};
use std::collections::{HashMap, HashSet};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

fn make_temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rustclaw_quantity_pair_binding_{label}_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&path).expect("temp root");
    path
}

fn test_state_with_root(root: PathBuf) -> AppState {
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            workspace_root: root.clone(),
            default_locator_search_dir: root,
            locator_scan_max_depth: 2,
            locator_scan_max_files: 100,
            tools_policy: Arc::new(
                crate::ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
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

fn executable_filename_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "read README and summarize".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: crate::OutputLocatorKind::Filename,
            locator_hint: "README.md".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    }
}

#[test]
fn quantity_compare_prebinds_two_workspace_directories_from_current_request() {
    let root = make_temp_root("quantity_dir_pair_prebind");
    std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case")).expect("right");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "bundle_src vs dynamic_guard_unpack_case".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.contains("bundle_src"));
    assert!(route
        .output_contract
        .locator_hint
        .contains("dynamic_guard_unpack_case"));

    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn recent_scalar_directory_pair_from_resolved_intent_promotes_to_quantity_comparison() {
    let root = make_temp_root("recent_scalar_dir_pair_promote");
    let left = root.join("fixtures/device_local/docs");
    let right = root.join("fixtures/device_local/logs");
    std::fs::create_dir_all(&left).expect("left directory");
    std::fs::create_dir_all(&right).expect("right directory");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;

    let resolved = format!(
        "Compare {} direct child count (3) with {} direct child count (2) and decide the larger one.",
        left.display(),
        right.display()
    );
    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(&state, &resolved, &mut route,)
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::QuantityComparison
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.contains("docs"));
    assert!(route.output_contract.locator_hint.contains("logs"));
    assert!(route
        .route_reason
        .contains("recent_scalar_directory_pair_promoted_to_quantity_comparison"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn recent_scalar_field_pair_from_current_request_does_not_promote_to_quantity_comparison() {
    let root = make_temp_root("recent_scalar_field_pair_no_quantity_promote");
    std::fs::create_dir_all(root.join("UI")).expect("ui directory");
    std::fs::create_dir_all(root.join("crates/clawd")).expect("clawd directory");
    std::fs::write(
        root.join("UI/package.json"),
        r#"{"name":"react-example","version":"0.0.0"}"#,
    )
    .expect("package json");
    std::fs::write(
        root.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
version = "0.1.0"
"#,
    )
    .expect("cargo toml");

    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;

    assert!(!prebind_quantity_compare_directory_pair_from_current_request(
        &state,
        "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，最后只用一行输出：前者、后者、一样或不一样",
        &mut route,
    ));

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RecentScalarEqualityCheck
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route
        .route_reason
        .contains("recent_scalar_directory_pair_promoted_to_quantity_comparison"));
    assert!(!route
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quantity_comparison_structured_field_pair_does_not_prebind_path_pair() {
    let root = make_temp_root("quantity_field_pair_no_path_prebind");
    std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local"))
        .expect("fixture directory");
    std::fs::create_dir_all(root.join("crates/clawd")).expect("clawd directory");
    std::fs::write(
        root.join("scripts/nl_tests/fixtures/device_local/package.json"),
        r#"{"name":"device-local"}"#,
    )
    .expect("package json");
    std::fs::write(
        root.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("cargo toml");

    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;

    assert!(!prebind_quantity_compare_directory_pair_from_current_request(
        &state,
        "Read package.name from scripts/nl_tests/fixtures/device_local/package.json and package.name from crates/clawd/Cargo.toml, then output the two names and equality verdict.",
        &mut route,
    ));

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::QuantityComparison
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quantity_compare_prebinds_two_workspace_files_from_surface_pair() {
    let root = make_temp_root("quantity_file_pair_prebind");
    std::fs::write(root.join("Cargo.lock"), "lock-data").expect("left");
    std::fs::write(root.join("Cargo.toml"), "toml").expect("right");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我前者大概是后者的几倍",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.contains("Cargo.lock"));
    assert!(route.output_contract.locator_hint.contains("Cargo.toml"));
    assert!(route
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quantity_compare_prebinds_bare_and_nested_workspace_directory_paths() {
    let root = make_temp_root("quantity_bare_nested_dir_pair_prebind");
    std::fs::create_dir_all(root.join("crates/skills")).expect("nested directory");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "count how many entries are directly under crates, then count how many are under crates/skills, and explain the layout",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.contains("crates"));
    assert!(route.output_contract.locator_hint.contains("crates/skills"));
    assert!(route
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quantity_compare_does_not_replace_single_existing_directory_locator_with_scanned_pair() {
    let root = make_temp_root("quantity_single_dir_no_pair_override");
    std::fs::create_dir_all(root.join("prompts/schemas")).expect("schemas");
    std::fs::create_dir_all(root.join("patches/open-lark/src/service/search/v2/schema"))
        .expect("other schema dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = root.join("prompts/schemas").display().to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        !prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "列出 prompts/schemas 目录下所有 .json 文件，并找出最大的 schema",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.locator_hint,
        root.join("prompts/schemas").display().to_string()
    );
    assert!(!route
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebinds_missing_locator_without_forcing_semantic_kind() {
    let root = make_temp_root("directory_pair_missing_locator_prebind");
    std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case")).expect("right");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .output_contract
        .locator_hint
        .contains("fixtures/tmp/bundle_src"));
    assert!(route
        .output_contract
        .locator_hint
        .contains("fixtures/tmp/dynamic_guard_unpack_case"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebind_skips_archive_locator_pair_contract() {
    let root = make_temp_root("directory_pair_archive_locator_pair");
    std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/tmp"))
        .expect("fixture dirs");
    std::fs::create_dir_all(root.join("tmp/contract_matrix_unpacked")).expect("dest dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(!prebind_quantity_compare_directory_pair_from_current_request(
        &state,
        concat!(
            "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。",
            "\n[CONTRACT_TEST_HINT]\n",
            "candidate_wrong_action_ref=fs_basic.write_text\n",
            "policy_expectation=runtime_must_reject_or_replace_disallowed_action\n",
            "[/CONTRACT_TEST_HINT]"
        ),
        &mut route,
    ));

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route
        .route_reason
        .contains("directory_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebind_skips_explicit_content_excerpt_contract() {
    let root = make_temp_root("directory_pair_content_excerpt");
    std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/docs"))
        .expect("fixture dirs");
    std::fs::create_dir_all(root.join(".git/objects/20")).expect("numeric dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;

    assert!(!prebind_quantity_compare_directory_pair_from_current_request(
        &state,
        concat!(
            "读取 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md 前 20 行，并用三句话总结。",
            "\n[CONTRACT_TEST_HINT]\n",
            "preferred_action_ref=archive_basic.read\n",
            "policy_expectation=use_allowed_action_with_required_evidence\n",
            "[/CONTRACT_TEST_HINT]"
        ),
        &mut route,
    ));

    assert_eq!(
        route.output_contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
    assert!(!route
        .route_reason
        .contains("directory_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebind_scan_reaches_late_structural_directory_tokens() {
    let root = make_temp_root("directory_pair_late_structural_scan");
    for idx in 0..2500 {
        std::fs::create_dir_all(root.join(format!("aaa_filler_{idx:04}"))).expect("filler");
    }
    std::fs::create_dir_all(root.join("zz_fixture/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("zz_fixture/tmp/dynamic_guard_unpack_case")).expect("right");
    let mut state = test_state_with_root(root.clone());
    state.skill_rt.locator_scan_max_files = 10;
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route
        .output_contract
        .locator_hint
        .contains("zz_fixture/tmp/bundle_src"));
    assert!(route
        .output_contract
        .locator_hint
        .contains("zz_fixture/tmp/dynamic_guard_unpack_case"));

    let _ = std::fs::remove_dir_all(root);
}
