use super::*;

#[test]
fn compression_preserves_exact_unicode_nul_and_indexed_fields() {
    let raw=serde_json::to_string(&json!({"event_kind":"tool_finished","event_hash":"stable","payload":{"skill":"example","requested_action_ref":"example.read","text":"中\0🙂".repeat(4096)}})).unwrap();
    let packed = encode(&raw, true).unwrap();
    assert_eq!(packed.version, 2);
    assert!(packed.json.len() + packed.blob.as_ref().unwrap().len() < raw.len() / 4);
    assert_eq!(
        decode(&packed.json, 2, packed.blob.as_deref()).unwrap(),
        raw
    );
    let indexed: Value = serde_json::from_str(&packed.json).unwrap();
    assert_eq!(indexed["payload"]["skill"], "example");
    assert_eq!(indexed["payload"]["requested_action_ref"], "example.read");
    assert!(decode("{}", 2, packed.blob.as_deref()).is_err());
}

#[test]
fn marker_in_untrusted_plain_payload_is_never_interpreted_as_storage() {
    let raw = r#"{"_event_storage":{"codec":"claimed-by-user","body":"untrusted"}}"#;
    assert_eq!(decode(raw, 0, None).unwrap(), raw);
    assert_eq!(decode(raw, 1, None).unwrap(), raw);
    assert_eq!(encode(raw, false).unwrap().version, 1);
    assert!(decode(raw, 99, None).is_err());
}

#[test]
fn corrupt_digest_codec_length_and_compression_bombs_fail_closed() {
    let packed = encode(&json!({"text":"a".repeat(100_000)}).to_string(), false).unwrap();
    let original = packed.blob.as_ref().unwrap();
    for offset in [0, 4, 13, 44, 46] {
        let mut blob = original.clone();
        blob[offset] ^= 0xff;
        assert!(
            decode(&packed.json, 2, Some(&blob)).is_err(),
            "offset={offset}"
        );
    }
    for length in [20_u64, MAX_PACKED_JSON_BYTES as u64 + 1] {
        let mut blob = original.clone();
        blob[5..13].copy_from_slice(&length.to_le_bytes());
        assert!(decode(&packed.json, 2, Some(&blob)).is_err());
    }
    let mut blob = original.clone();
    blob.extend_from_slice(b"tail");
    assert!(decode(&packed.json, 2, Some(&blob)).is_err());
    assert!(decode(&packed.json, 2, None).is_err());
}
