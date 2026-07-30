use super::{ProductIdentity, ProductIdentityConfig};

fn assert_identity_fixture(raw: &str) {
    let config: ProductIdentityConfig = toml::from_str(raw).expect("parse identity fixture");
    let identity = ProductIdentity::from_config(&config).expect("validate identity fixture");
    assert_eq!(identity.display_name(), config.display_name.trim());
    assert_eq!(
        identity.release_artifact_id(),
        config.release_artifact_id.trim()
    );
    assert_eq!(identity.terminal_banner(), config.terminal_banner.trim());
    assert_eq!(
        identity.release_repository(),
        config.release_repository.trim()
    );
    assert_eq!(
        identity.small_screen_splash_image(),
        config.small_screen_splash_image.trim()
    );
}

#[test]
fn config_changes_identity_without_source_changes() {
    assert_identity_fixture(include_str!(
        "../../../scripts/fixtures/product_identity/brand-primary.toml"
    ));
}

#[test]
fn a_second_brand_uses_the_same_runtime_code_path() {
    assert_identity_fixture(include_str!(
        "../../../scripts/fixtures/product_identity/brand-alternate.toml"
    ));
}

#[test]
fn invalid_identity_values_are_rejected_instead_of_using_code_defaults() {
    let mut config: ProductIdentityConfig = toml::from_str(include_str!(
        "../../../scripts/fixtures/product_identity/brand-primary.toml"
    ))
    .expect("parse identity fixture");
    config.release_artifact_id = "INVALID VALUE".to_string();
    assert!(ProductIdentity::from_config(&config).is_err());
}
