use super::*;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn make_temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rustclaw_locator_resolution_{label}_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&path).expect("temp root");
    path
}

fn route_with_locator(locator_kind: crate::OutputLocatorKind) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
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
            locator_kind,
            ..Default::default()
        },
    }
}

#[test]
fn auto_locator_attempts_for_path_locators_even_without_content_evidence() {
    let mut route = route_with_locator(crate::OutputLocatorKind::Path);
    route.resolved_intent = "读取 Cargo.toml".to_string();
    route.output_contract.requires_content_evidence = false;
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_attempts_for_current_workspace_locator() {
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.resolved_intent = "检查当前目录是否存在隐藏文件".to_string();
    route.output_contract.requires_content_evidence = false;
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_attempts_for_filename_locators() {
    let mut route = route_with_locator(crate::OutputLocatorKind::Filename);
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = "读取 README 前 20 行".to_string();
    route.output_contract.requires_content_evidence = true;
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_skips_non_path_locators() {
    let mut route = route_with_locator(crate::OutputLocatorKind::None);
    route.resolved_intent = "今天天气".to_string();
    route.output_contract.requires_content_evidence = false;
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_skips_clarify_routes() {
    let mut route = route_with_locator(crate::OutputLocatorKind::Path);
    route.ask_mode = crate::AskMode::clarify_trace();
    route.resolved_intent = "读一下那个 README 开头，然后一句话总结".to_string();
    route.needs_clarify = true;
    route.route_reason = "normalizer requested clarification before execution".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_skips_stateful_ordered_entry_clarify_routes() {
    let mut route = route_with_locator(crate::OutputLocatorKind::Filename);
    route.ask_mode = crate::AskMode::clarify_trace();
    route.resolved_intent = "看第二个".to_string();
    route.needs_clarify = true;
    route.route_reason =
        "stateful_ordered_entry_ambiguous_clarify:content_read:entries=4".to_string();
    route.route_confidence = Some(0.97);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_skips_clarify_with_unbound_workspace_scope() {
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.ask_mode = crate::AskMode::clarify_trace();
    route.resolved_intent = "检查当前目录".to_string();
    route.needs_clarify = true;
    route.output_contract.requires_content_evidence = true;
    assert!(!should_attempt_auto_locator(&route));

    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "docs".to_string();
    assert!(should_attempt_auto_locator(&route));

    route.output_contract.locator_hint.clear();
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn quantity_comparison_current_workspace_without_hint_does_not_auto_locator_to_root() {
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.route_reason = "quantity_comparison".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    assert!(!should_attempt_auto_locator(&route));

    route.output_contract.locator_hint = "/tmp/repo/target".to_string();
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn current_workspace_locator_resolution_prefers_workspace_root() {
    let root = make_temp_root("current_workspace_locator_root");
    std::fs::create_dir_all(root.join("rustclaw")).expect("nested rustclaw dir");
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = "Write a long introduction for RustClaw".to_string();
    route.route_reason = "workspace summary".to_string();
    route.output_contract.requires_content_evidence = true;

    assert!(matches!(
        current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_locator_resolution_accepts_absolute_workspace_hint() {
    let root = make_temp_root("current_workspace_locator_abs_root");
    std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("launcher file");
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = "Introduce RustClaw as the current project".to_string();
    route.route_reason = "workspace summary".to_string();
    route.output_contract.locator_hint = root.display().to_string();
    route.output_contract.requires_content_evidence = true;

    assert!(matches!(
        current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_locator_hint_naming_root_resolves_to_workspace_root() {
    let parent = make_temp_root("current_workspace_locator_root_name");
    let root = parent.join("rustclaw");
    std::fs::create_dir_all(&root).expect("workspace root");
    std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("same-name child");
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = "Introduce the current RustClaw project".to_string();
    route.route_reason = "workspace summary".to_string();
    route.output_contract.locator_hint = "RustClaw".to_string();
    route.output_contract.requires_content_evidence = true;

    assert!(matches!(
        current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn current_workspace_locator_hint_with_target_name_does_not_resolve_to_root() {
    let root = make_temp_root("current_workspace_locator_named_hint");
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.resolved_intent = "列出 archive 目录下的所有条目".to_string();
    route.output_contract.locator_hint = "archive".to_string();
    route.output_contract.requires_content_evidence = true;

    assert!(current_workspace_locator_resolution(&root, &route).is_none());
    assert_eq!(
        effective_auto_locator_kind(&route),
        crate::OutputLocatorKind::Path
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_empty_locator_hint_resolves_to_root() {
    let root = make_temp_root("current_workspace_locator_empty_hint");
    let mut route = route_with_locator(crate::OutputLocatorKind::CurrentWorkspace);
    route.resolved_intent = "列出当前工作区".to_string();
    route.output_contract.requires_content_evidence = true;

    assert!(matches!(
        current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    assert_eq!(
        effective_auto_locator_kind(&route),
        crate::OutputLocatorKind::CurrentWorkspace
    );
    let _ = std::fs::remove_dir_all(root);
}
