use super::*;

#[test]
fn direct_capability_request_accepts_only_machine_contract() {
    let parsed = parse_direct_capability_request(&serde_json::json!({
        "entrypoint": "run_capability",
        "capability": "workspace.diff",
        "args": {"checkpoint_id": "checkpoint_1"}
    }))
    .expect("parse direct capability");
    assert_eq!(parsed.capability, "workspace.diff");
    assert_eq!(parsed.args["checkpoint_id"], "checkpoint_1");

    assert!(parse_direct_capability_request(&serde_json::json!({
        "entrypoint": "run_capability",
        "capability": "workspace diff",
        "args": {}
    }))
    .is_err());
    assert!(parse_direct_capability_request(&serde_json::json!({
        "entrypoint": "run_capability",
        "capability": "workspace.diff",
        "args": "not-an-object"
    }))
    .is_err());
    assert!(parse_direct_capability_request(&serde_json::json!({
        "capability": "workspace.diff",
        "args": {}
    }))
    .is_err());
}
