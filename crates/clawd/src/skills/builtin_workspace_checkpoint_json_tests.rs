use super::*;

#[test]
fn checkpoint_json_exact_limits_count_serialized_utf8_and_escapes() {
    for value in [
        serde_json::json!({}),
        serde_json::json!({"text":"plain"}),
        serde_json::json!({"text":"\u{4e2d}\u{6587}\u{001f}\n\"\\"}),
    ] {
        let expected = serde_json::to_vec_pretty(&value).unwrap();
        assert_eq!(encode_pretty(&value, expected.len()).unwrap(), expected);
        for limit_bytes in [0, expected.len() - 1] {
            assert!(matches!(encode_pretty(&value, limit_bytes),
                Err(EncodeError::LimitExceeded { limit_bytes: actual }) if actual == limit_bytes));
        }
    }
}

#[test]
fn checkpoint_json_size_limits_do_not_hide_a_serializer_failure() {
    struct Refuses;
    impl Serialize for Refuses {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("owned_serializer_refusal"))
        }
    }
    for limit in [0, 1024] {
        assert!(matches!(
            encode_pretty(&Refuses, limit),
            Err(EncodeError::Serialization(_))
        ));
    }
}

#[test]
fn checkpoint_json_growth_and_larger_budgets_preserve_pretty_bytes() {
    let value = serde_json::json!({"items":(0..100).collect::<Vec<_>>()});
    let expected = serde_json::to_vec_pretty(&value).unwrap();
    assert_eq!(encode_pretty(&value, usize::MAX).unwrap(), expected);
    assert!(matches!(
        encode_pretty(&value, expected.len() / 2),
        Err(EncodeError::LimitExceeded { .. })
    ));
}
