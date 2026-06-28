use super::requested_machine_kv_summary_from_observations;

#[test]
fn machine_summary_accepts_grounded_command_with_path_continuation() {
    let observed =
        vec!["144|Use the auto-sync script: `python3 scripts/sync_skill_docs.py`.".to_string()];

    let summary = requested_machine_kv_summary_from_observations(
        "Answer exactly as machine summary: command=python3 scripts/sync_skill_docs.py.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("command=python3 scripts/sync_skill_docs.py")
    );
}

#[test]
fn machine_summary_accepts_inline_machine_enums_and_observed_script() {
    let observed =
        vec!["212|When changing Rust code, run `python3 scripts/check_long_files.py`.".to_string()];

    let summary = requested_machine_kv_summary_from_observations(
        "Answer exactly: hard_ceiling_lines=2000 script=check_long_files.py.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("hard_ceiling_lines=2000 script=check_long_files.py")
    );
}

#[test]
fn machine_summary_accepts_comma_list_machine_literals() {
    let observed = vec![
        "Prefer registry_metadata,INTERFACE.md,generated_prompts over clawd_main_flow.".to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Only answer: prefer=registry_metadata,INTERFACE.md,generated_prompts over=clawd_main_flow.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("prefer=registry_metadata,INTERFACE.md,generated_prompts over=clawd_main_flow")
    );
}

#[test]
fn machine_summary_accepts_nested_machine_token_value() {
    let observed = vec![
        "88|- `kind=run_skill` does not run the intent normalizer or planner / agent loop."
            .to_string(),
        "95|| Does it enter the planner / agent loop? | Yes. | No.".to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Only answer: run_skill=kind=run_skill planner=No.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("run_skill=kind=run_skill planner=No")
    );
}

#[test]
fn machine_summary_preserves_dotted_markers_and_embedded_pairs() {
    let observed = vec![
        "task_control.resume.dry_run task_control.pause.dry_run checkpoint_id=ckpt-1 task_id=00000000-0000-4000-8000-000000000010 pause_seconds=120 would_mutate=false"
            .to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Preview task_control.resume(checkpoint_id=ckpt-1) and task_control.pause(pause_seconds=120). Final must contain task_control.resume.dry_run task_control.pause.dry_run and checkpoint_id. task_id=00000000-0000-4000-8000-000000000010",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("task_control.resume.dry_run task_control.pause.dry_run task_id=00000000-0000-4000-8000-000000000010 checkpoint_id=ckpt-1 pause_seconds=120")
    );
}

#[test]
fn single_inline_flag_pair_still_requires_observed_value() {
    let observed = vec!["This line does not contain the requested flag.".to_string()];

    let summary =
        requested_machine_kv_summary_from_observations("Only answer: required=yes.", &observed);

    assert!(summary.is_none());
}
