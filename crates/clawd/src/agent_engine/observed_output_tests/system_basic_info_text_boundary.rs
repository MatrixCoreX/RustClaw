#[test]
fn system_basic_info_accepts_machine_extra_payload() {
    let body = r#"{
        "status": "ok",
        "text": "basic system info",
        "extra": {
            "hostname": "rustclaw-host",
            "os": "linux",
            "cwd": "/home/guagua/rustclaw"
        }
    }"#;

    let value = super::system_basic_info_value("system_basic", body).expect("machine info payload");
    assert_eq!(
        value.get("hostname").and_then(|v| v.as_str()),
        Some("rustclaw-host")
    );
    assert_eq!(value.get("os").and_then(|v| v.as_str()), Some("linux"));
}

#[test]
fn system_basic_info_ignores_json_hidden_in_visible_text() {
    let body = r#"{
        "status": "ok",
        "text": "{\"hostname\":\"rustclaw-host\",\"os\":\"linux\",\"cwd\":\"/home/guagua/rustclaw\"}"
    }"#;

    assert!(super::system_basic_info_value("system_basic", body).is_none());
}
