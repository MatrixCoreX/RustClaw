use super::*;

#[test]
fn request_payload_round_trips_after_serialized_restart_without_sidecars() {
    let payload = json!({"messages":[{"role":"system","content":"fixture context ".repeat(10000)}],"tools":[{"name":"fixture"}]});
    let encoded = encode(&payload);
    assert_eq!(encoded["model_log_encoding"], ENCODING);
    assert!(encoded.to_string().len() < payload.to_string().len() / 5);
    let restarted = serde_json::from_str(&encoded.to_string()).unwrap();
    assert_eq!(decode(restarted).unwrap(), payload);
    assert_eq!(
        decode(json!({"messages":[]})).unwrap(),
        json!({"messages":[]})
    );
    assert_eq!(encode(&Value::Null), Value::Null);
}

#[test]
fn corrupted_or_expanding_payloads_fail_closed() {
    let encoded = encode(&json!({"messages":"context ".repeat(10000)}));
    for (field, value) in [
        ("model_log_encoding", json!("unknown")),
        ("uncompressed_bytes", json!(MAX_BYTES + 1)),
        ("uncompressed_bytes", json!(1)),
        ("sha256", json!("invalid")),
        ("data", json!("invalid base64")),
    ] {
        let mut corrupt = encoded.clone();
        corrupt[field] = value;
        assert!(decode(corrupt).is_err(), "field={field}");
    }
    let mut truncated = encoded;
    let data = truncated["data"].as_str().unwrap().to_string();
    truncated["data"] = json!(&data[..data.len() / 2]);
    assert!(decode(truncated).is_err());
}
