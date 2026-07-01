#[test]
fn structured_listing_detection_accepts_machine_extra_payload() {
    let value = serde_json::json!({
        "status": "ok",
        "text": "directory inventory",
        "extra": {
            "action": "inventory_dir",
            "names": ["src", "Cargo.toml"]
        }
    });

    assert!(super::super::execution_summary::structured_listing_observation_for_test(&value));
}

#[test]
fn structured_listing_detection_ignores_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "action": "inventory_dir",
        "names": ["src", "Cargo.toml"]
    })
    .to_string();
    let value = serde_json::json!({
        "status": "ok",
        "text": hidden_payload
    });

    assert!(!super::super::execution_summary::structured_listing_observation_for_test(&value));
}
