use super::*;

#[test]
fn resource_diagnostics_remain_during_wait_but_not_after_resume_or_completion() {
    let original = json!({"resume_reason":"resource_admission_wait","message_key":"clawd.task.resource_waiting", "resource_wait":{"schema_version":1,"reason_code":"storage_pressure"}});
    let mut waiting = original.as_object().unwrap().clone();
    append_lifecycle_product_contract_fields(&mut waiting, &json!({}), "waiting");
    assert!(waiting.contains_key("resource_wait"));
    for state in ["running", "succeeded", "failed"] {
        let mut current = original.as_object().unwrap().clone();
        append_lifecycle_product_contract_fields(&mut current, &json!({}), state);
        assert!(!current.contains_key("resource_wait"));
    }
    let mut current = original.as_object().unwrap().clone();
    current.insert("resume_reason".to_string(), json!("pending_async_job"));
    append_lifecycle_product_contract_fields(&mut current, &json!({}), "background");
    assert!(!current.contains_key("resource_wait"));
}

#[test]
fn null_resume_reason_does_not_hide_a_new_authoritative_reason_code() {
    let mut current=json!({"resume_reason":null,"reason_code":"pending_async_job","message_key":"clawd.task.resource_waiting","resource_wait":{"schema_version":1,"reason_code":"storage_pressure"}}).as_object().unwrap().clone();
    append_lifecycle_product_contract_fields(&mut current, &json!({}), "background");
    assert!(!current.contains_key("resource_wait"));
}
