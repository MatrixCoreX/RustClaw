use super::OfficeError;
use serde_json::json;

#[test]
fn argument_replan_error_exposes_generic_recovery_contract() {
    let extra = OfficeError::replan_argument(
        "invalid_cursor",
        "cursor does not match the source",
        json!({"cursor": "stale"}),
        "cursor",
    )
    .extra();

    assert_eq!(extra["error_code"], "invalid_cursor");
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["recovery_action"], "replan_arguments");
    assert_eq!(extra["invalid_argument"], "cursor");
}
