use super::*;

#[test]
fn output_policy_loads_generation_limits_without_changing_task_ceilings() {
    let root = toml::from_str(
        "[agent.task_budget.model_output]\ndefault_tokens=6000\nverifier_tokens=2000\nrecovery_max_tokens=9000\nrecovery_attempts=0\n",
    )
    .unwrap();
    let policy = ModelOutputBudgetPolicy::from_toml(&root);
    assert_eq!(policy.stage_tokens("plan"), 6000);
    assert_eq!(policy.stage_tokens("verifier"), 2000);
    assert_eq!(policy.recovery_max_tokens, 9000);
    assert_eq!(policy.recovery_attempts, 0);
}

#[test]
fn invalid_generation_values_keep_valid_defaults() {
    let root = toml::from_str(
        "[agent.task_budget.model_output]\ndefault_tokens=0\nverifier_tokens=-1\nrecovery_max_tokens='bad'\nrecovery_attempts=-1\n",
    )
    .unwrap();
    assert_eq!(
        ModelOutputBudgetPolicy::from_toml(&root),
        ModelOutputBudgetPolicy::default()
    );
}
