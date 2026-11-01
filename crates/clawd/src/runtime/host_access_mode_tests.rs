use super::*;

#[test]
fn ordinary_access_modes_round_trip_without_privileged_bits() {
    for bits in 0..=0o777 {
        let mode = HostAccessMode::try_from(bits).unwrap();
        assert_eq!(u32::from(mode), bits);
        assert_eq!(serde_json::to_value(mode).unwrap(), bits);
        assert_eq!(
            serde_json::from_value::<HostAccessMode>(bits.into()).unwrap(),
            mode
        );
    }
}

#[test]
fn malformed_or_privileged_modes_cannot_enter_the_typed_boundary() {
    for value in [
        serde_json::json!(-1),
        serde_json::json!(0o1000),
        serde_json::json!(0o4755),
        serde_json::json!(0o2755),
        serde_json::json!(u64::MAX),
        serde_json::json!("700"),
        serde_json::json!(null),
        serde_json::json!(1.5),
    ] {
        assert!(serde_json::from_value::<HostAccessMode>(value).is_err());
    }
}
