use tempfile::tempdir;

use super::*;

#[test]
fn package_scan_rejects_private_keys_without_echoing_them() {
    let root = tempdir().expect("tempdir");
    let secret = "-----BEGIN PRIVATE KEY-----\nfixture-secret\n-----END PRIVATE KEY-----\n";
    std::fs::write(root.path().join("leaked.pem"), secret).expect("secret fixture");
    let error = scan_package_source(root.path()).expect_err("secret must be rejected");
    assert_eq!(error.code, "package_secret_detected");
    assert!(!error.detail.contains("fixture-secret"));
}

#[test]
fn diagnostics_redact_sensitive_assignments() {
    let redacted = redact_diagnostics("building\nAPI_TOKEN=super-secret-value\nfinished");
    assert!(redacted.contains("building"));
    assert!(redacted.contains("[redacted sensitive diagnostic]"));
    assert!(!redacted.contains("super-secret-value"));
}
