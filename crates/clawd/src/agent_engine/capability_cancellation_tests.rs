use super::*;

fn mapping(source: &str) -> PlannerCapabilityMapping {
    toml::from_str(source).expect("capability mapping")
}

#[test]
fn cancellation_classes_follow_machine_contracts_not_skill_names() {
    let observe = mapping(
        r#"
name = "fixture.inspect"
effect = "observe"
execution_mode = "sync_short"
"#,
    );
    let mutate = mapping(
        r#"
name = "fixture.apply"
effect = "mutate"
execution_mode = "sync_short"
"#,
    );
    let async_job = mapping(
        r#"
name = "fixture.start"
effect = "external"
execution_mode = "async_required"
async_adapter_kind = "http_job_poll"
"#,
    );

    assert_eq!(
        classify_mapping(
            Some(&observe),
            SkillKind::Runner,
            false,
            false,
            "process_runner"
        )
        .class,
        CapabilityCancellationClass::ReadOnly
    );
    assert_eq!(
        classify_mapping(Some(&mutate), SkillKind::Builtin, true, true, "builtin").class,
        CapabilityCancellationClass::CooperativeMutation
    );
    assert_eq!(
        classify_mapping(
            Some(&mutate),
            SkillKind::Runner,
            false,
            true,
            "process_runner"
        )
        .class,
        CapabilityCancellationClass::ReconciliationRequiredMutation
    );
    assert_eq!(
        classify_mapping(
            Some(&async_job),
            SkillKind::Runner,
            false,
            true,
            "process_runner"
        )
        .class,
        CapabilityCancellationClass::SupervisedExternalJob
    );
}

#[test]
fn mutation_cancel_projection_never_claims_external_rollback() {
    let contract = classify_mapping(None, SkillKind::Runner, false, true, "process_runner");
    let projection = contract.projection("fixture", Some("publish"));

    assert_eq!(projection["local_execution_state"], "stopped");
    assert_eq!(projection["external_effect_state"], "outcome_unknown");
    assert_eq!(projection["settlement_state"], "reconciliation_required");
    assert_eq!(
        projection["late_result_policy"],
        "record_without_resuming_old_plan"
    );
}
