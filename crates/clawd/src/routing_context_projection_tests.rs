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

#[test]
fn execution_arguments_keep_locators_and_field_selections_but_preview_stays_thin() {
    let args = json!({"path":"README.md","url":"https://example.test/post/1",
        "field_paths":["/request","/capabilities/0/args"],"mode":"head","n":1});
    let (actual, truncated, complete) = bounded_execution_argument_value(&args);
    assert_eq!(actual, Some(args));
    assert!(!truncated);
    assert_eq!(complete, ["field_paths", "mode", "n", "path", "url"]);
    assert_eq!(
        bounded_prior_context_value(&json!({"path":"README.md","n":1})),
        (Some(json!({"n":1})), false)
    );
}

#[test]
fn large_argument_body_does_not_hide_complete_locator_or_claim_a_complete_object() {
    let args = json!({"content":"x".repeat(1024),"path":"src/example.py",
        "topics":(0..20).collect::<Vec<_>>(),"empty":null});
    let (actual, truncated, complete) = bounded_execution_argument_value(&args);
    assert!(truncated);
    assert_eq!(actual.as_ref().unwrap()["path"], "src/example.py");
    assert!(actual.as_ref().unwrap().get("content").is_none());
    assert_eq!(complete, ["empty", "path"]);
    assert_eq!(actual.unwrap()["topics"].as_array().unwrap().len(), 16);
}

#[test]
fn stored_arguments_keep_empty_shapes_and_never_capture_authority_or_private_artifact_paths() {
    let args = json!({"empty":{},"items":[],"optional":null,"public_key":"public-fixture",
        "accessToken":"hidden-fixture","confirmation":true,
        "path":".agent-runtime/artifacts/private-file"});
    let (actual, truncated, complete) = bounded_execution_argument_value(&args);
    assert!(truncated);
    assert_eq!(
        actual,
        Some(json!({"empty":{},"items":[],"optional":null,"public_key":"public-fixture"}))
    );
    assert_eq!(complete, ["empty", "items", "optional", "public_key"]);
}
