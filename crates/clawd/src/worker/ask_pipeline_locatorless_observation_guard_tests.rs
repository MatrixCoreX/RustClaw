use super::{
    locatorless_observation_route_should_force_clarify,
    promote_locatorless_git_capability_to_repository_state,
    promote_locatorless_scalar_child_metadata_to_quantity_comparison,
};
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
        "rustclaw_locatorless_observation_guard_{label}_{}_{}",
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
fn locatorless_observation_requires_clarify_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_requires_clarify"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Read the first 3 lines of the referenced file.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state,
        "read the first 3 lines of that file",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_git_capability_promotes_to_repository_state_before_clarify_guards() {
    let state = test_state_with_root(make_temp_root("locatorless_git_capability"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Observe git repository state from the current workspace.".to_string();
    route.route_reason = "This requires git_basic readonly observation.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(promote_locatorless_git_capability_to_repository_state(
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GitRepositoryState
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state, "git", &route, None, &snapshot,
    ));
}

#[test]
fn locatorless_scalar_child_metadata_promotes_to_quantity_comparison() {
    let root = make_temp_root("locatorless_scalar_child_metadata");
    std::fs::create_dir_all(root.join("target")).expect("target dir");
    std::fs::write(root.join("target").join("artifact.bin"), b"artifact").expect("artifact");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(
        promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state,
            "inspect target size",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::QuantityComparison
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.join("target")
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
    assert!(!route.needs_clarify);
}

#[test]
fn locatorless_rss_news_fetch_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_rss_news_fetch"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Fetch the latest configured RSS news.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RssNewsFetch;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "fetch latest rss news",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_web_search_summary_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_web_search_summary"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Search the web for rust async tutorial and summarize results.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WebSearchSummary;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "search rust async tutorial",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_weather_query_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_weather_query"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Query current weather for Beijing.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WeatherQuery;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "check Beijing weather",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_market_quote_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_market_quote"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Query the latest quote for 600519.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::MarketQuote;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "check 600519 quote",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_image_understanding_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_image_understanding"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Describe the attached image.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ImageUnderstanding;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "describe the attached image",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_publishing_preview_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_publishing_preview"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Draft a channel post preview and do not publish it.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::PublishingPreview;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "draft a channel post preview without publishing",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_scalar_raw_runtime_observation_can_plan_without_state_patch() {
    let state = test_state_with_root(make_temp_root("locatorless_runtime_scalar_raw"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "runtime scalar status query",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_inline_structured_payload_does_not_require_external_locator() {
    let state = test_state_with_root(make_temp_root("locatorless_inline_payload"));
    let mut route = executable_filename_route();
    route.resolved_intent = r#"Count inline JSON array records."#.to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2}]"#,
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_git_capability_repairs_command_summary_contract_to_repository_state() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Observe Git repository state from the current workspace.".to_string();
    route.route_reason = "normalizer selected git_repository_state capability".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.exact_sentence_count = Some(1);

    assert!(promote_locatorless_git_capability_to_repository_state(
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GitRepositoryState
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::OneSentence
    );
    assert_eq!(route.output_contract.exact_sentence_count, Some(1));
}

#[test]
fn visible_git_skill_candidate_does_not_steal_process_status_route() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Observe local listening ports from the host system.".to_string();
    route.route_reason = "This requires local process status observation.".to_string();
    route.visible_skill_candidates = vec!["process_basic".to_string(), "git_basic".to_string()];
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    assert!(!promote_locatorless_git_capability_to_repository_state(
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
}

#[test]
fn locatorless_raw_file_content_does_not_promote_to_quantity_comparison() {
    let root = make_temp_root("locatorless_raw_file_content");
    std::fs::write(root.join("README.md"), b"line one\nline two\n").expect("readme");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;

    assert!(
        !promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state,
            "read README now",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert!(route.output_contract.locator_hint.is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn locatorless_one_sentence_file_content_does_not_promote_to_quantity_comparison() {
    let root = make_temp_root("locatorless_one_sentence_file_content");
    std::fs::write(root.join("README.md"), b"line one\nline two\n").expect("readme");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;

    assert!(
        !promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state,
            "read README now",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn command_payload_raw_output_does_not_promote_to_quantity_comparison() {
    let root = make_temp_root("command_payload_not_quantity_comparison");
    std::fs::create_dir_all(root.join("run")).expect("run dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;

    assert!(
        !promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state,
            "please run uname -a and tell me the result",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn locatorless_scalar_child_metadata_preserves_bare_single_token_topic() {
    let root = make_temp_root("locatorless_scalar_bare_topic");
    std::fs::create_dir_all(root.join("target")).expect("target dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(
        !promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state, "target", &mut route,
        )
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn locatorless_raw_command_output_still_allows_execution() {
    let mut state = test_state_with_root(make_temp_root("locatorless_raw_command"));
    state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "execute pwd, then explain what the path means in one sentence",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_sequence_output_still_allows_execution() {
    let mut state = test_state_with_root(make_temp_root("locatorless_raw_command_sequence"));
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];
    let mut route = executable_filename_route();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "请依次执行 pwd 和 whoami，直接输出两个命令结果，每个结果一行，不要总结",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_without_explicit_command_requires_clarify() {
    let state = test_state_with_root(make_temp_root("locatorless_raw_without_command"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state,
        "list directory contents",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_observation_allows_active_structured_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_active_anchor"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/work/README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "再看前 3 行",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_transform_requires_structured_input_despite_unstructured_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_raw_command_input"));
    let mut route = executable_filename_route();
    route.resolved_intent = "transform /tmp/work/README.md as structured data".to_string();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let prompt = "把那个 JSON 数组按 score 排成表格";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state, prompt, &route, None, &snapshot,
    ));
}
