use super::*;
use serde_json::json;

#[test]
fn api_recovery_codes_remain_structured_skill_errors_not_success_or_prose() {
    for token in [
        "context_source_compacted",
        "context_field_compacted",
        "context_snapshot_changed",
        "context_cursor_expired",
        "context_reference_unauthorized",
    ] {
        let output = decode_response(
            false,
            crate::ApiResponse {
                ok: false,
                data: None,
                error: Some(token.into()),
            },
        )
        .unwrap();
        assert_eq!(output.extra.unwrap()["error_code"], token);
        assert_eq!(output.error_text.as_deref(), Some(token));
    }
    let output = decode_response(
        false,
        crate::ApiResponse {
            ok: false,
            data: None,
            error: Some("raw unexpected body".into()),
        },
    )
    .unwrap();
    assert_eq!(
        output.extra.unwrap()["error_code"],
        "context_recall_api_failed"
    );
}

#[test]
fn pagination_options_preserve_opaque_values_and_validate_types() {
    let digest = "a".repeat(64);
    let args = json!({"field_paths":["/capabilities"], "cursor":"opaque.cursor", "page_size":3, "source_digest":digest});
    let options = Options::parse(&args).unwrap();
    assert_eq!(options.cursor.as_deref(), Some("opaque.cursor"));
    assert_eq!(options.page_size, Some(3));
    assert_eq!(options.source_digest.as_deref(), Some(digest.as_str()));
    for (key, value) in [
        ("cursor", json!(12)),
        ("cursor", json!("x".repeat(1025))),
        ("page_size", json!(0)),
        ("page_size", json!(65)),
        ("page_size", json!(-1)),
        ("page_size", json!(1.5)),
        ("source_digest", json!("invalid")),
    ] {
        let mut invalid = args.clone();
        invalid[key] = value;
        assert!(Options::parse(&invalid).is_err(), "{key}");
    }
    assert!(Options::parse(&json!({"field_paths":["/request"],"page_size":1})).is_err());
    assert!(Options::parse(&json!({"field_paths":["/request"]})).is_ok());
}
