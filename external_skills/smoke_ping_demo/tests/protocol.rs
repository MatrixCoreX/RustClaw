#[test]
fn protocol_contract_declares_structured_machine_errors() {
    let source = include_str!("../src/main.rs");
    assert!(source.contains("error_code"));
    assert!(source.contains("message_key"));
}
