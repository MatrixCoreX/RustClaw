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
fn single_inline_flag_pair_still_requires_observed_value() {
    let observed = vec!["This line does not contain the requested flag.".to_string()];

    let summary =
        requested_machine_kv_summary_from_observations("Only answer: required=yes.", &observed);

    assert!(summary.is_none());
}
