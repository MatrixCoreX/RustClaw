use serde_json::{json, Value};

#[test]
fn persisted_trace_scores_round_trip_without_changing_float_bits() {
    for value in [0.43886807560920715_f64, 0.45377013087272644_f64] {
        let original = json!({"memory_trace": {"score": value}});
        let bytes = serde_json::to_vec(&original).unwrap();
        let restored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            restored["memory_trace"]["score"]
                .as_f64()
                .unwrap()
                .to_bits(),
            value.to_bits()
        );
        assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    }
}

#[test]
fn finite_json_numbers_keep_exact_bits_through_persisted_replay() {
    let mut bits = 0x1234_5678_9abc_def0_u64;
    let extremes = [
        0.0,
        -0.0,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
    ];
    for value in extremes
        .into_iter()
        .chain((0..4096).map(|_| {
            bits = bits.wrapping_mul(6364136223846793005).wrapping_add(1);
            f64::from_bits(bits)
        }))
        .filter(|value| value.is_finite())
    {
        let bytes = serde_json::to_vec(&json!(value)).unwrap();
        let restored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            restored.as_f64().unwrap().to_bits(),
            value.to_bits(),
            "{value:?}"
        );
        assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    }
}

#[test]
fn numeric_replay_preserves_integer_and_decimal_string_contracts() {
    let original = json!({"unsigned":u64::MAX,"signed":i64::MIN,"decimal":"0.12345678"});
    let bytes = serde_json::to_vec(&original).unwrap();
    let restored: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored["unsigned"].as_u64(), Some(u64::MAX));
    assert_eq!(restored["signed"].as_i64(), Some(i64::MIN));
    assert_eq!(restored["decimal"].as_str(), Some("0.12345678"));
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
}
