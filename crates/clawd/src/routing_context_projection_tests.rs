use super::*;

#[test]
fn bounded_previews_explicitly_report_object_array_depth_and_byte_omissions() {
    for value in [
        json!((0..17).collect::<Vec<_>>()),
        json!((0..33)
            .map(|i| (format!("k{i:02}"), json!(i)))
            .collect::<serde_json::Map<_, _>>()),
        json!({"text":"x".repeat(513)}),
        json!({"a":{"b":{"c":{"d":{"e":{"f":1}}}}}}),
        json!({"items":vec!["x".repeat(500);16]}),
    ] {
        let (_, truncated) = bounded_prior_context_value(&value);
        assert!(truncated);
    }
    let value = json!({"a":3,"items":[1,2,3]});
    assert_eq!(bounded_prior_context_value(&value), (Some(value), false));
}

#[test]
fn redaction_preserves_preview_limits_without_exposing_secrets_or_private_paths() {
    let value = json!({"count":1,"api_key":"hidden", "path":"/private/file"});
    assert_eq!(
        bounded_prior_context_value(&value),
        (Some(json!({"count":1})), false)
    );
}
