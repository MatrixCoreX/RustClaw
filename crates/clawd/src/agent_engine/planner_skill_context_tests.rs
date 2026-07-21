use super::*;
use serde_json::json;

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "planner-skill-context-test".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("planner-skill-context-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn state() -> crate::AppState {
    crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry()
}

fn full_visible_playbook_chars(state: &crate::AppState, task: &crate::ClaimedTask) -> usize {
    state
        .planner_available_skills_for_task(task)
        .into_iter()
        .filter_map(|skill| state.skill_registry_prompt_rel_path(&skill))
        .map(|path| crate::load_prompt_template_for_state(state, &path, "").0)
        .map(|text| text.chars().count())
        .sum()
}

fn plan_with_step(action_type: &str, skill: &str, args: serde_json::Value) -> crate::PlanResult {
    crate::PlanResult {
        goal: "fixture_goal".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: action_type.to_string(),
            skill: skill.to_string(),
            args,
            depends_on: Vec::new(),
            why: "fixture".to_string(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: "{}".to_string(),
    }
}

fn loop_state_with_plan(plan: crate::PlanResult) -> super::super::LoopState {
    let mut loop_state = super::super::LoopState {
        round_no: 2,
        ..Default::default()
    };
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: plan.goal.clone(),
            execution_recipe_summary: None,
            plan_result: Some(plan),
            verify_result: None,
        });
    loop_state
}

#[test]
fn first_round_uses_only_budgeted_compact_index() {
    let state = state();
    let task = task();
    let loop_state = super::super::LoopState {
        round_no: 1,
        ..Default::default()
    };
    let context = build_planner_skill_context(&state, &task, &loop_state);

    assert_eq!(context.disclosure_mode, "compact_index");
    assert!(context.selected_skills.is_empty());
    assert!(context.text.contains("runtime_skill_context_v2"));
    assert!(context.text.contains("Compact skill index:"));
    assert!(!context.text.contains("Selected skill playbooks:"));
    assert!(context.quick_index_chars <= SKILL_QUICK_INDEX_CHAR_BUDGET);
    assert_eq!(context.playbook_chars, 0);
    assert!(context.text.chars().count() < full_visible_playbook_chars(&state, &task));
    let task_control_line = context
        .text
        .lines()
        .find(|line| line.contains("skill=task_control"));
    assert!(
        context
            .text
            .contains("side-effect-free coding-repair previews"),
        "task_control_line={task_control_line:?}"
    );
    assert!(context.text.contains("coding_workflow.preview_repair"));
    let task_control_line = task_control_line.expect("task_control compact-index line");
    assert!(
        task_control_line.contains("allowed_failure_class="),
        "task_control_line={task_control_line}"
    );
    assert!(
        task_control_line.contains("quota_exhausted"),
        "task_control_line={task_control_line}"
    );
    let run_cmd_line = context
        .text
        .lines()
        .find(|line| line.contains("skill=run_cmd"))
        .expect("run_cmd compact-index line");
    assert!(
        run_cmd_line.contains("system.preview_background_command"),
        "run_cmd_line={run_cmd_line}"
    );
    let system_basic_line = context
        .text
        .lines()
        .find(|line| line.contains("skill=system_basic"))
        .expect("system_basic compact-index line");
    assert!(
        system_basic_line.contains("system.runtime_status(action=runtime_status,required=kind"),
        "system_basic_line={system_basic_line}"
    );
    assert!(
        system_basic_line.contains("allowed_kind=current_time|current_user|current_working_directory|host_name|kernel_release"),
        "system_basic_line={system_basic_line}"
    );
    assert!(
        system_basic_line
            .contains("allowed_sort_by=mtime_asc|mtime_desc|name|name_desc|size_asc|size_desc"),
        "system_basic_line={system_basic_line}"
    );
    let fs_basic_line = context
        .text
        .lines()
        .find(|line| line.contains("skill=fs_basic"))
        .expect("fs_basic compact-index line");
    for capability in [
        "filesystem.stat_paths",
        "filesystem.list_entries",
        "filesystem.read_text_range",
        "filesystem.find_entries",
        "filesystem.grep_text",
        "filesystem.write_file",
        "workspace.apply_patch",
        "workspace.review_child_patch",
    ] {
        assert!(
            fs_basic_line.contains(capability),
            "missing capability={capability}; fs_basic_line={fs_basic_line}"
        );
    }
    let video_line = context
        .text
        .lines()
        .find(|line| line.contains("skill=video_generate"))
        .expect("video_generate compact-index line");
    assert!(
        video_line.contains(
            "optional=first_frame_image|first_frame|image|last_frame_image|last_frame|duration|resolution|output_path"
        ),
        "video_line={video_line}"
    );
    let schedule_line = context
        .text
        .lines()
        .find(|line| line.contains("skill=schedule"))
        .expect("schedule compact-index line");
    assert!(
        !schedule_line.contains("summary=Shared skill prompt contract:"),
        "schedule_line={schedule_line}"
    );
    assert!(
        schedule_line.contains("schedule.preview(action=preview"),
        "schedule_line={schedule_line}"
    );
}

#[test]
fn generated_prompt_summary_prefers_capability_content_over_role_boilerplate() {
    let prompt = r#"
## Role & Boundaries
- You are the `demo` skill planner.

## Capability Summary (from interface)
- Observe a machine contract without side effects.

## Actions
- `preview`
"#;

    assert_eq!(
        first_non_heading_line(prompt).as_deref(),
        Some("- Observe a machine contract without side effects.")
    );
}

#[test]
fn layered_prompt_summary_prefers_capability_section_over_common_preamble() {
    let prompt = r#"
Shared skill prompt contract:
- Common rules that apply to every skill.

## schedule — schedule semantic compiler

## Capability
- Compile scheduling requests into structured plans.
"#;

    assert_eq!(
        first_non_heading_line(prompt).as_deref(),
        Some("- Compile scheduling requests into structured plans.")
    );
}

#[test]
fn later_round_expands_playbook_from_structured_capability_only() {
    let state = state();
    let loop_state = loop_state_with_plan(plan_with_step(
        "call_capability",
        "filesystem.list_entries",
        json!({"path": "."}),
    ));
    let context = build_planner_skill_context(&state, &task(), &loop_state);

    assert_eq!(context.disclosure_mode, "scoped_playbooks");
    assert_eq!(context.selected_skills, vec!["fs_basic".to_string()]);
    assert!(context.text.contains("Selected skill playbooks:"));
    assert!(context.text.contains("### fs_basic"));
    assert!(context.playbook_chars <= SKILL_PLAYBOOK_CHAR_BUDGET);
}

#[test]
fn user_visible_response_text_never_selects_a_skill_playbook() {
    let state = state();
    let loop_state = loop_state_with_plan(plan_with_step(
        "respond",
        "respond",
        json!({"content": "fs_basic filesystem.list_entries"}),
    ));
    let context = build_planner_skill_context(&state, &task(), &loop_state);

    assert_eq!(context.disclosure_mode, "compact_index");
    assert!(context.selected_skills.is_empty());
    assert!(!context.text.contains("Selected skill playbooks:"));
}
