use super::initial_plan_for_round;

#[test]
fn seeded_direct_plan_is_used_on_the_first_restored_round() {
    let plan = crate::PlanResult {
        goal: "checkpoint replay".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: Vec::new(),
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: "{}".to_string(),
    };

    assert!(initial_plan_for_round(4, 4, Some(&plan)).is_some());
    assert!(initial_plan_for_round(5, 4, Some(&plan)).is_none());
    assert!(initial_plan_for_round(4, 4, None).is_none());
}
