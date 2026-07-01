use super::{
    background_only_locator_route_should_force_clarify,
    downgrade_background_locator_clarify_to_recent_observed_chat,
    route_has_model_supplied_concrete_locator,
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
        "rustclaw_background_locator_guard_{label}_{}_{}",
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
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
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
fn background_only_locator_rewrite_requires_clarify_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("background_locator_requires_clarify"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "读取 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md 前 3 行"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "读一下那个文件前 3 行",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_locator_guard_ignores_embedded_answer_candidate_path() {
    let state = test_state_with_root(make_temp_root("background_answer_candidate_path"));
    let mut route = executable_filename_route();
    route.resolved_intent = format!(
        "Return the current runtime scalar\nanswer_candidate: {}",
        state.skill_rt.workspace_root.display()
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let resolved_prompt = route.resolved_intent.clone();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!route_has_model_supplied_concrete_locator(
        &route,
        &resolved_prompt
    ));
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "return the runtime scalar",
        &resolved_prompt,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn generated_file_delivery_runtime_target_bypasses_background_locator_clarify() {
    let state = test_state_with_root(make_temp_root(
        "background_generated_delivery_runtime_target",
    ));
    let mut route = executable_filename_route();
    route.resolved_intent = "/tmp/hello.sh".to_string();
    route.route_reason = "generated_file_delivery_allows_runtime_target".to_string();
    route.wants_file_delivery = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(route_has_model_supplied_concrete_locator(
        &route,
        &route.resolved_intent
    ));
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "Create a shell script that prints hello world, save it, and send me the file.",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn model_supplied_manifest_locator_from_deictic_prompt_requires_clarify() {
    let root = make_temp_root("background_manifest_locator_requires_clarify");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").expect("manifest");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Extract the name field from the package manifest (Cargo.toml).".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "extract name from that package file",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_only_extensionless_file_locator_with_current_request_evidence_can_plan() {
    let root = make_temp_root("background_locator_extensionless_file");
    std::fs::write(root.join("README.md"), "# Demo\n").expect("readme");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Read the beginning of README and summarize it.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "读一下那个 README 开头并用一句话总结",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_only_field_selector_without_file_locator_requires_clarify() {
    let root = make_temp_root("background_locator_field_selector");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config_path = config_dir.join("app_config.toml");
    std::fs::write(&config_path, "[app]\nname = \"Demo\"\n").expect("config");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Extract app.name from a remembered config file.".to_string();
    route.route_reason =
        "single_path_config_field_extraction_contract_semantically_valid".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.display().to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "去那个配置里找 app.name，只把值给我",
        "去那个配置里找 app.name，只把值给我",
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_only_locator_rewrite_allows_active_ordered_anchor() {
    let state = test_state_with_root(make_temp_root("background_locator_anchor"));
    let mut route = executable_filename_route();
    route.resolved_intent = "读取 /tmp/work/crates/larkd 前 3 行".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/crates/larkd".to_string();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/work/crates".to_string()),
            ordered_entries: vec!["larkd".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "看最后一个的基本信息",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_only_locator_rewrite_allows_recent_ordered_entry_anchor() {
    let state = test_state_with_root(make_temp_root("background_locator_recent_ordered"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Send the selected prior listing entry: logs/clawd-dev.log".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd-dev.log".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
    let recent_execution_context = "\
### RECENT_EXECUTION_EVENTS
- ts=2 kind=ask request=list document result=builtin_write_smoke.txt
full_suite_trace_note.txt
gen-1778122040.png
- ts=1 kind=ask request=list logs result=act_plan.log, clawd-dev.log, clawd.log, clawd.nl-focus.log";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/work/document".to_string()),
            ordered_entries: vec!["builtin_write_smoke.txt".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "send the selected entry",
        &route.resolved_intent,
        recent_execution_context,
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_locator_clarify_keeps_file_name_judgment_without_machine_marker() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.clarify_question = "missing target".to_string();
    route.resolved_intent =
        "Compare two recently observed file excerpts and return the selected filename".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/service_notes.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.route_reason =
        "semantic judgment already resolved; background_locator_requires_clarify".to_string();
    let recent_execution_context = "\
### RECENT_EXECUTION_EVENTS
- ts=2 kind=ask request=read README.md result=# RustClaw
Chinese version: README.zh-CN.md
- ts=1 kind=ask request=read service_notes.md result=# Service Notes
RustClaw test fixture service notes.";

    assert!(
        !downgrade_background_locator_clarify_to_recent_observed_chat(
            &mut route,
            recent_execution_context,
        )
    );
    assert!(route.needs_clarify);
    assert_eq!(route.ask_mode.gate_kind(), crate::RouteGateKind::Clarify);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
    );
    assert!(!route
        .route_reason
        .contains("active_observed_output_chat_repair"));
}

#[test]
fn background_locator_clarify_downgrades_existing_observed_synthesis_with_recent_result() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.clarify_question = "missing target".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/act_plan.log".to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.route_reason = concat!(
        "existing_observed_context_synthesis; ",
        "clarify_reason_code:missing_target; ",
        "background_locator_requires_clarify"
    )
    .to_string();
    let recent_execution_context = "\
### RECENT_EXECUTION_EVENTS
- ts=2 kind=ask request=tail act_plan.log result={\"phase\":\"loop_done\",\"tool_calls\":1}";

    assert!(
        downgrade_background_locator_clarify_to_recent_observed_chat(
            &mut route,
            recent_execution_context,
        )
    );
    assert!(!route.needs_clarify);
    assert_eq!(route.ask_mode.gate_kind(), crate::RouteGateKind::Chat);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::OneSentence
    );
    assert!(route
        .route_reason
        .contains("active_observed_output_chat_repair"));
}
