use crate::capability_request::RequestedEffect;
use crate::PackageManifest;

fn manifest() -> PackageManifest {
    PackageManifest::from_toml_str(include_str!(
        "../../../optional_skills/media_download/skill.toml"
    ))
    .expect("maintained package with account controls")
}

#[test]
fn aipp_account_controls_are_optional_and_roundtrip_without_self_grants() {
    let mut current = manifest();
    let controls = current
        .aipp
        .as_ref()
        .unwrap()
        .account_controls
        .as_ref()
        .unwrap();
    assert_eq!(controls.schema_version, 1);
    assert_eq!(controls.status_capability, "media_download.account_status");
    let encoded = current.to_toml_string().unwrap();
    assert_eq!(PackageManifest::from_toml_str(&encoded).unwrap(), current);
    current.aipp.as_mut().unwrap().account_controls = None;
    assert!(
        current.validate().is_ok(),
        "old immutable packages remain valid"
    );
}

#[test]
fn aipp_account_controls_reject_foreign_missing_or_misclassified_capabilities() {
    for invalid in [
        "foreign.account_logout",
        "media_download.missing",
        "media_download.account_status",
    ] {
        let mut current = manifest();
        current
            .aipp
            .as_mut()
            .unwrap()
            .account_controls
            .as_mut()
            .unwrap()
            .logout_capability = invalid.into();
        assert_eq!(
            current.validate().unwrap_err().code,
            "manifest_aipp_account_controls_invalid"
        );
    }
    let mut current = manifest();
    let action = current
        .capability_request
        .as_mut()
        .unwrap()
        .capabilities
        .iter_mut()
        .find(|action| action.name.ends_with(".account_logout"))
        .unwrap();
    action.effect = RequestedEffect::Observe;
    assert_eq!(
        current.validate().unwrap_err().code,
        "manifest_aipp_account_controls_invalid"
    );
    let mut current = manifest();
    let action = current
        .capability_request
        .as_mut()
        .unwrap()
        .capabilities
        .iter_mut()
        .find(|action| action.name.ends_with(".account_logout"))
        .unwrap();
    action.required.retain(|field| field != "confirm");
    assert_eq!(
        current.validate().unwrap_err().code,
        "manifest_aipp_account_controls_invalid"
    );
}
