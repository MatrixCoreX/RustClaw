use super::*;

#[test]
fn direct_scalar_observed_answer_extracts_markdown_heading_from_read_range() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist","path":"release_checklist.md"}"#,
    ));
    let route = scalar_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, _) =
        direct_scalar_observed_answer(None, &loop_state, Some(&ctx)).expect("heading answer");

    assert_eq!(answer, "Release Checklist");
    assert!(!should_attach_execution_summary(
        &loop_state,
        Some(&ctx),
        Some("Read the note file title and output only the title.")
    ));

    let mut route = scalar_route_result();
    route.requires_content_evidence = false;
    route.locator_kind = OutputLocatorKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let (answer, _) =
        direct_scalar_observed_answer(None, &loop_state, Some(&ctx)).expect("heading answer");
    assert_eq!(answer, "Release Checklist");
    assert!(!should_attach_execution_summary(
        &loop_state,
        Some(&ctx),
        Some("Read the note file title and output only the title.")
    ));
}

#[test]
fn markdown_heading_direct_scalar_defers_when_read_evidence_has_body() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    assert!(markdown_heading_from_read_output(
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#
    )
    .is_none());
}

#[test]
fn direct_scalar_observed_answer_skips_separator_markdown_heading() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# =========================\n2|# Image Edit","path":"configs/image.toml"}"#,
    ));
    let route = scalar_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, _) =
        direct_scalar_observed_answer(None, &loop_state, Some(&ctx)).expect("heading answer");
    assert_eq!(answer, "Image Edit");
}
#[test]
fn observed_markdown_heading_scalar_replaces_repaired_strict_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "note file".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Release Checklist".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["# Release Checklist".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["# Release Checklist".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_keeps_locatorless_strict_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::None;
    route.locator_hint.clear();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Release Checklist".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["# Release Checklist".to_string()]);
    assert!(summary.is_none());
}

#[test]
fn observed_markdown_heading_scalar_replaces_ungrounded_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Service Notes".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["# Service Notes".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["# Service Notes".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_replaces_one_sentence_locator_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["Service Notes".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["Service Notes".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["Service Notes".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_suppresses_summary_for_free_locator_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["Service Notes".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["Service Notes".to_string()]);
    assert!(summary.is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);
    assert_eq!(delivery, vec!["Service Notes".to_string()]);
}

#[test]
fn observed_markdown_heading_scalar_reduces_strict_observed_markdown_body() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(
        delivery,
        vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()]
    );
    assert!(summary.is_none());
}

#[test]
fn observed_markdown_heading_scalar_reduces_scalar_wrapped_observed_markdown_body() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.\n4|2. Confirm database migrations are applied.","path":"release_checklist.md","resolved_path":"/repo/release_checklist.md"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|# Release Checklist\\n2|\\n3|1. Verify configuration loads correctly.\\n4|2. Confirm database migrations are applied.\",\"path\":\"release_checklist.md\"}"}"#,
    ));
    let mut route = scalar_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "release_checklist.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec![
        "# Release Checklist\n\n1. Verify configuration loads correctly.\n2. Confirm database migrations are applied."
            .to_string(),
    ];
    let mut summary = None;

    assert!(replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["Release Checklist".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("Release Checklist")
    );
    assert!(summary.is_some());
}

#[test]
fn observed_markdown_heading_scalar_strips_heading_marker_for_title_selector() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = scalar_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md".to_string();
    route.self_extension.structured_field_selector = Some("title".to_string());
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Service Notes".to_string()];
    let mut summary = None;

    assert!(replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["Service Notes".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("Service Notes")
    );
    assert!(summary.is_some());
}

#[test]
fn observed_markdown_heading_scalar_replaces_bounded_strict_wrapper_from_observed_heading() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "release_checklist.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["자료A의 제목은 'Release Checklist'입니다.".to_string()];
    let mut summary = None;

    assert!(replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(delivery, vec!["Release Checklist".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("Release Checklist")
    );
    assert!(summary.is_some());
}

#[test]
fn observed_markdown_heading_scalar_keeps_heading_prefixed_summary_without_selector() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "release_checklist.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["Release Checklist covers deployment checks.".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(
        delivery,
        vec!["Release Checklist covers deployment checks.".to_string()]
    );
    assert!(summary.is_none());
}

#[test]
fn direct_scalar_observed_answer_prefers_observed_heading_for_title_selector() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "respond",
        "자료A의 제목은 'Release Checklist'입니다.",
    ));
    let mut route = scalar_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "release_checklist.md".to_string();
    route.self_extension.structured_field_selector = Some("title".to_string());
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&ctx)).expect("heading answer");

    assert_eq!(answer, "Release Checklist");
    assert_eq!(
        summary.stage,
        Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric)
    );
}

#[test]
fn observed_markdown_heading_scalar_keeps_free_observed_markdown_body() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes.","path":"service_notes.md"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery = vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));

    assert_eq!(
        delivery,
        vec!["# Service Notes\n\nRustClaw test fixture service notes.".to_string()]
    );
    assert!(summary.is_none());
}
