use super::{
    LimitClass, LimitHit, LimitHitValidationError, LimitRecovery, LimitUnit,
    LIMIT_HIT_SCHEMA_VERSION,
};

fn resumable_hit() -> LimitHit {
    LimitHit {
        schema_version: LIMIT_HIT_SCHEMA_VERSION,
        class: LimitClass::TaskResource,
        owner: "task_budget_manager".to_string(),
        unit: LimitUnit::Calls,
        configured_value: 256,
        observed_value: 256,
        reason_code: "administrator_budget_exhausted".to_string(),
        terminal: false,
        recovery: LimitRecovery::CheckpointRequeue,
    }
}

#[test]
fn resumable_limit_hit_has_one_machine_readable_owner_and_recovery() {
    let hit = resumable_hit();
    hit.validate().unwrap();
    let encoded = serde_json::to_value(hit).unwrap();
    assert_eq!(encoded["class"], "task_resource");
    assert_eq!(encoded["recovery"], "checkpoint_requeue");
}

#[test]
fn non_terminal_limit_hit_requires_recovery() {
    let mut hit = resumable_hit();
    hit.recovery = LimitRecovery::None;
    assert_eq!(
        hit.validate(),
        Err(LimitHitValidationError::MissingRecovery)
    );
}

#[test]
fn terminal_safety_limit_may_fail_closed_without_automatic_recovery() {
    let mut hit = resumable_hit();
    hit.class = LimitClass::Safety;
    hit.terminal = true;
    hit.recovery = LimitRecovery::None;
    hit.reason_code = "workspace_confinement_denied".to_string();
    hit.validate().unwrap();
}
