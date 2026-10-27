use super::*;

#[test]
fn execution_verification_and_delivery_are_reported_independently() {
    let input = json!({"result_json":{"task_journal":{"summary":{
        "execution_status":"succeeded", "answer_verification":{"schema_version":1,"status":"unavailable"},
        "delivery":{"schema_version":1,"status":"partial"}
    }}}});
    let report = task_outcome_report_json(&input, &json!({}));
    assert_eq!(report["execution_status"], "succeeded");
    assert_eq!(report["answer_verification"]["status"], "unavailable");
    assert_eq!(report["delivery"]["status"], "partial");
}

#[test]
fn absent_legacy_facts_do_not_claim_a_verified_answer() {
    let report = task_outcome_report_json(&json!({"status":"succeeded"}), &json!({}));
    assert!(report["answer_verification"].is_null());
    assert!(report["execution_status"].is_null());
    assert!(report["delivery"].is_null());
}
