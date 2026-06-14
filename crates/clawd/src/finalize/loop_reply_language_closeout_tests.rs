use super::*;

#[test]
fn execution_recipe_closeout_note_mentions_external_workspace_for_english_code_change() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        saw_external_target: true,
        ..Default::default()
    };

    let note = execution_recipe_closeout_note(
        None,
        "Fix the issue in /tmp/demo and verify it.",
        &loop_state,
    )
    .expect("closeout note");
    assert!(note.contains("external workspace"));
    assert!(note.contains("code changes"));
}

#[test]
fn execution_recipe_closeout_prefixes_greenfield_plain_text_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        saw_greenfield_creation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["Validation passed.".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "Create a new script and verify it works.",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].starts_with("Created the new artifact"));
    assert!(delivery[0].ends_with("Validation passed."));
}

#[test]
fn execution_recipe_closeout_does_not_infer_success_marker_from_user_text() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let mut delivery = vec!["修复已经完成。".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "修复系统服务并在通过时明确输出 VALIDATION_PASSED。",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].contains("系统范围"));
    assert!(!delivery[0].contains("VALIDATION_PASSED"));
    assert!(delivery[0].ends_with("修复已经完成。"));
}

#[test]
fn execution_recipe_closeout_prefixes_current_repo_plain_text_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["修复已经验证通过。".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "把当前仓库里的问题修好并验证。",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].starts_with("已在当前仓库完成代码修改"));
    assert!(delivery[0].ends_with("修复已经验证通过。"));
}

#[test]
fn execution_recipe_closeout_note_mentions_system_scope_for_english_ops() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };

    let note = execution_recipe_closeout_note(
        None,
        "Repair the system service and validate it.",
        &loop_state,
    )
    .expect("closeout note");
    assert!(note.contains("system scope"));
    assert!(note.contains("ops work"));
}

#[test]
fn execution_recipe_closeout_note_skips_apply_phase_without_validation() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };

    assert!(execution_recipe_closeout_note(
        None,
        "Repair the system service and validate it.",
        &loop_state,
    )
    .is_none());
}

#[test]
fn execution_recipe_closeout_skips_file_token_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        validation_required: true,
        saw_validation: true,
        saw_external_target: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["FILE:/tmp/report.txt".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "Update the config in another workspace and verify it.",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["FILE:/tmp/report.txt".to_string()]);
}

#[test]
fn execution_recipe_closeout_skips_scalar_route_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        validation_required: true,
        saw_validation: true,
        saw_external_target: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["42".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "Fix the value in /tmp/demo and just answer with the number.",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["42".to_string()]);
}

#[test]
fn execution_recipe_closeout_skips_scalar_route_when_marker_is_only_user_text() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let mut delivery = vec!["VALIDATION_PASSED".to_string()];

    attach_execution_recipe_closeout_to_delivery(
        None,
        "修复当前仓库问题，通过时明确输出 VALIDATION_PASSED。",
        &loop_state,
        Some(&ctx),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["VALIDATION_PASSED".to_string()]);
}

#[test]
fn ensure_requested_success_marker_visible_does_not_scan_user_text() {
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let mut delivery = vec!["Completed ops work at the system scope and validated it.".to_string()];

    ensure_requested_success_marker_visible(Some(&ctx), &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert!(delivery[0].contains("system scope"));
    assert!(!delivery[0].contains("VALIDATION_PASSED"));
}

#[test]
fn missing_requested_success_marker_does_not_scan_user_text() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["ops-repair-bad".to_string()];
    assert_eq!(
        missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}

#[test]
fn requested_success_marker_allows_recipe_success_when_present() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["VALIDATION_PASSED".to_string()];
    assert_eq!(
        missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}

#[test]
fn auto_requested_success_marker_stays_off_without_structured_request() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
    assert_eq!(
        auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}

#[test]
fn auto_requested_success_marker_stays_off_before_recipe_done() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };
    let ctx = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "When it passes, explicitly output VALIDATION_PASSED and stop immediately.".to_string(),
        ),
        ..Default::default()
    };
    let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
    assert_eq!(
        auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
        None
    );
}
