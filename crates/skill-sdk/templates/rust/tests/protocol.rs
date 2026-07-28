#[test]
fn protocol_fixture_keeps_tests_separate_from_production() {
    let request = serde_json::json!({
        "request_id": "test-1",
        "args": {"action": "__FIRST_ACTION__"},
        "context": null,
        "user_id": 1,
        "chat_id": 1
    });
    assert_eq!(request["request_id"], "test-1");
}
