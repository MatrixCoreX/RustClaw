use super::*;
use serde_json::json;

#[test]
fn execution_evidence_redacts_credentials_and_authority_without_dropping_public_locators() {
    let input = json!({"path":"README.md","url":"https://example.test/post/1",
        "public_key":"public-fixture","assetPublicKey":"public-fixture",
        "api_key":"hidden-one","apiKey":"hidden-two","privateKey":"hidden-three",
        "accessToken":"hidden-four","clientSecret":"hidden-five",
        "permissionMode":"admin","approval_token":"hidden-six",
        "runtimeUserKey":"hidden-nine","providerApiKey":"hidden-ten",
        "__runtime_control":true,
        "headers":{"Authorization":"hidden-seven"},"nested":{"password":"hidden-eight","n":1}});
    assert_eq!(
        redact(&input, 0).unwrap(),
        json!({
        "path":"README.md","url":"https://example.test/post/1",
        "public_key":"public-fixture","assetPublicKey":"public-fixture","nested":{"n":1}})
    );
}

#[test]
fn argument_redaction_preserves_null_empty_and_scalar_shapes() {
    let input = json!({"a":null,"b":[],"c":{},"d":false,"e":0});
    assert_eq!(redact(&input, 0).unwrap(), input);
    let mut deep = json!(1);
    for _ in 0..14 {
        deep = json!({"nested":deep});
    }
    assert_eq!(redact(&deep, 0), Err("context_value_too_deep"));
}
