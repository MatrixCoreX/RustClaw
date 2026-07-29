use tempfile::tempdir;

use super::*;
use crate::PackageManifest;

#[test]
fn every_language_scaffold_has_a_valid_manifest_and_separate_tests() {
    for language in [
        ImplementationLanguage::Rust,
        ImplementationLanguage::Python,
        ImplementationLanguage::Node,
        ImplementationLanguage::Go,
        ImplementationLanguage::Prebuilt,
        ImplementationLanguage::GenericProcess,
        ImplementationLanguage::HttpJson,
    ] {
        let root = tempdir().expect("tempdir");
        let destination = root.path().join(language.as_token());
        let outcome = scaffold_skill(&ScaffoldRequest {
            destination: destination.clone(),
            skill_name: format!("demo_{}", language.as_token()),
            capability_summary: "Return one structured demo result.".to_string(),
            actions: vec!["run".to_string()],
            implementation_language: language,
            source_root: ".".to_string(),
        })
        .expect("scaffold");
        let manifest = PackageManifest::load(&outcome.manifest_path).expect("manifest");
        assert_eq!(manifest.package.name, outcome.skill_name);
        assert_eq!(
            manifest.schema_version,
            crate::manifest::SKILL_MANIFEST_SCHEMA_VERSION
        );
        manifest
            .effective_capability_request()
            .expect("typed capability request")
            .validate()
            .expect("valid capability request");
        let self_grant = std::fs::read_to_string(&outcome.manifest_path)
            .expect("manifest source")
            .replace(
                "[capability_request]\n",
                "[capability_request]\nauto_invocable = true\n",
            );
        assert_eq!(
            PackageManifest::from_toml_str(&self_grant)
                .expect_err("package cannot self-grant auto invocation")
                .code,
            "manifest_parse_failed"
        );
        assert!(
            outcome
                .written_files
                .iter()
                .any(|path| path.to_string_lossy().contains("test"))
                || language == ImplementationLanguage::Prebuilt
        );
    }
}

#[test]
fn scaffold_rejects_unknown_language_and_nonempty_destination() {
    assert!(ImplementationLanguage::parse("ruby").is_err());
    let root = tempdir().expect("tempdir");
    std::fs::write(root.path().join("owned.txt"), "keep").expect("sentinel");
    let error = scaffold_skill(&ScaffoldRequest {
        destination: root.path().to_path_buf(),
        skill_name: "demo".to_string(),
        capability_summary: "demo".to_string(),
        actions: vec![],
        implementation_language: ImplementationLanguage::Python,
        source_root: ".".to_string(),
    })
    .expect_err("must refuse overwrite");
    assert_eq!(error.code, "scaffold_destination_not_empty");
}
