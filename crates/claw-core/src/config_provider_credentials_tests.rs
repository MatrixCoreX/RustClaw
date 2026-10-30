use super::{LlmCredentialSource, LlmProviderParams};

#[test]
fn credential_source_is_runtime_owned_not_deserializable_configuration() {
    for input in [
        "",
        "credential_source = 'resolved'",
        "credential_source = true",
        "credential_source = { variant = 'resolved' }",
    ] {
        let params: LlmProviderParams = toml::from_str(input).unwrap();
        assert_eq!(params.credential_source, LlmCredentialSource::Managed);
    }
    let params = LlmProviderParams {
        credential_source: LlmCredentialSource::Resolved,
        ..Default::default()
    };
    assert_eq!(
        params.clone().credential_source,
        LlmCredentialSource::Resolved
    );
}
