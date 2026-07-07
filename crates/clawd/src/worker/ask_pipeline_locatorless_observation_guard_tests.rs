use super::locatorless_observation_route_should_defer_to_agent_loop;
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
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
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
fn locatorless_observation_defers_to_agent_loop_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_defers_to_loop"));
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

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "read the first 3 lines of that file",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_kb_metadata_capability_can_plan_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_kb_metadata"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "capability_ref=kb.list_namespaces field=names field=count limit=10".to_string();
    route.route_reason = "capability_ref=kb.list_namespaces".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "list kb namespaces",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_kb_search_capability_can_plan_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_kb_search"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "capability_ref=kb.search namespace=docs query=service_status top_k=3".to_string();
    route.route_reason = "structured skill-managed KB search; capability_ref=kb.search".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "search kb namespace",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_registry_capability_ref_can_plan_without_guard_whitelist_growth() {
    let state = test_state_with_root(make_temp_root("locatorless_generic_capability_ref"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Generate a short media preview. capability_ref=media_video.generate".to_string();
    route.route_reason =
        "registry capability route; capability_ref=media_video.generate".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "generate a short media preview",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_registry_capability_ref_absent_from_route_reason_still_defers_to_loop() {
    let state = test_state_with_root(make_temp_root("locatorless_capability_ref_resolved_only"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Generate a short media preview. capability_ref=media_video.generate".to_string();
    route.route_reason = "registry capability route without machine ref".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "generate a short media preview",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_git_capability_plans_without_enum_promotion() {
    let state = test_state_with_root(make_temp_root("locatorless_git_capability"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Observe repository state from the current workspace. capability_ref=git_basic.status"
            .to_string();
    route.route_reason = "capability_ref=git_basic.status; readonly observation".to_string();
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

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state, "git", &route, None, &snapshot,
    ));
}

#[test]
fn locatorless_rss_news_fetch_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_rss_news_fetch"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Fetch the latest configured RSS news. capability_ref=rss.latest_news".to_string();
    route.route_reason = "capability_ref=rss.latest_news".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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
        "Search the web for rust async tutorial and summarize results. capability_ref=web.search_results"
            .to_string();
    route.route_reason = "capability_ref=web.search_results".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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
    route.resolved_intent =
        "Query current weather for Beijing. capability_ref=weather.current".to_string();
    route.route_reason = "capability_ref=weather.current".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "check Beijing weather",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_weather_query_without_capability_ref_defers_to_agent_loop() {
    let state = test_state_with_root(make_temp_root("locatorless_weather_no_capability"));
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

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "check Beijing weather",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn ordinary_semantic_kind_without_capability_ref_defers_to_agent_loop() {
    let state = test_state_with_root(make_temp_root(
        "locatorless_ordinary_semantic_no_capability",
    ));
    for semantic_kind in [
        crate::OutputSemanticKind::ServiceStatus,
        crate::OutputSemanticKind::HiddenEntriesCheck,
        crate::OutputSemanticKind::WorkspaceProjectSummary,
        crate::OutputSemanticKind::RecentScalarEqualityCheck,
        crate::OutputSemanticKind::GitCommitSubject,
        crate::OutputSemanticKind::GitRepositoryState,
        crate::OutputSemanticKind::ToolDiscovery,
    ] {
        let mut route = executable_filename_route();
        route.resolved_intent = format!("ordinary semantic contract: {}", semantic_kind.as_str());
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = semantic_kind;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(
            !locatorless_observation_route_should_defer_to_agent_loop(
                &state,
                "run ordinary semantic route without explicit locator",
                &route,
                None,
                &snapshot,
            ),
            "semantic_kind={}",
            semantic_kind.as_str()
        );
    }
}

#[test]
fn invalid_locatorless_capability_ref_token_defers_to_agent_loop() {
    let state = test_state_with_root(make_temp_root("locatorless_capability_token_shape"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Query current weather for Beijing. capability_ref=weathercurrent".to_string();
    route.route_reason = "capability_ref=weathercurrent".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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
    route.resolved_intent =
        "Query the latest quote for 600519. capability_ref=stock.quote".to_string();
    route.route_reason = "capability_ref=stock.quote".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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
    route.resolved_intent =
        "Describe the attached image. capability_ref=image_vision.describe".to_string();
    route.route_reason = "capability_ref=image_vision.describe".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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
    route.resolved_intent =
        "Draft a channel post preview and do not publish it. capability_ref=x.draft_preview"
            .to_string();
    route.route_reason = "capability_ref=x.draft_preview".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "draft a channel post preview without publishing",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_scalar_raw_runtime_observation_without_machine_signal_requires_clarify() {
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

    assert!(locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "runtime scalar status query",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_scalar_runtime_observation_with_machine_marker_can_plan() {
    let state = test_state_with_root(make_temp_root("locatorless_runtime_scalar_marker"));
    let mut route = executable_filename_route();
    route.route_reason = "execution_recipe_scalar_runtime_tool_observation".to_string();
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

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2}]"#,
        &route,
        None,
        &snapshot,
    ));
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

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state,
        "execute pwd, then explain what the path means in one sentence",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_explicit_command_uses_policy_evidence_without_semantic_enum() {
    let mut state = test_state_with_root(make_temp_root("locatorless_explicit_command_no_enum"));
    state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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

    assert!(locatorless_observation_route_should_defer_to_agent_loop(
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

    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
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

    assert!(locatorless_observation_route_should_defer_to_agent_loop(
        &state, prompt, &route, None, &snapshot,
    ));
}
