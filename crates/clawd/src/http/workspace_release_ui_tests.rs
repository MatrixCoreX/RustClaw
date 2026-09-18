use super::*;

#[test]
fn workspace_update_source_checkout_mode_preserves_installation_state() {
    let previous = WorkspaceUpdateStatus {
        installation_kind: "release_package".to_string(),
        source_update_available: false,
        ..WorkspaceUpdateStatus::default()
    };

    let started = begin_workspace_update_status(&previous, WorkspaceUpdateMode::SourceCheckout);

    assert_eq!(started.status, "running");
    assert_eq!(started.mode, "source_checkout");
    assert_eq!(started.installation_kind, "release_package");
    assert!(!started.source_update_available);
}

#[test]
fn workspace_update_release_lookup_errors_control_cached_tag_reuse() {
    assert!(LatestReleaseLookupError::RequestTimedOut.can_use_cached_tag());
    assert!(LatestReleaseLookupError::HttpStatus.can_use_cached_tag());
    assert!(!LatestReleaseLookupError::CompatibleReleaseNotFound.can_use_cached_tag());
    assert!(!LatestReleaseLookupError::UnsupportedPlatform.can_use_cached_tag());
    assert_eq!(
        LatestReleaseLookupError::CompatibleReleaseNotFound.as_str(),
        "compatible_release_not_found"
    );
}

#[test]
fn workspace_update_release_deploy_uses_stable_release_and_prebuilt_ui() {
    let script = include_str!("../../../../deploy-github-release.sh");
    assert!(script.contains("release.get(\"draft\") or release.get(\"prerelease\")"));
    assert!(script.contains("checksum_name = f\"{archive_name}.sha256\""));
    assert!(script.contains("release_checksum=verified"));
    assert!(script.contains("\"$ROOT_DIR/build-ui-nginx.sh\" --copy-if-configured"));
    assert!(script.contains("rollback_deployment"));
    assert!(script.contains("--package-mode"));
    assert!(script.contains("release_package_status=enabled"));
    assert!(script.contains("PACKAGE_MODE_ORIGINAL_MOVED"));
    assert!(script.contains("NEW_CONFIG_PATHS_FILE"));
    assert!(!script.contains("rm -rf data"));
    assert!(!script.contains("cp -a \"$PACKAGE_DIR/target/release/.\""));
    assert!(!script.contains("build-ui-nginx.sh --deploy-if-configured"));
    let source_checkout_script = include_str!("../../../../scripts/switch-to-source-checkout.sh");
    assert!(
        source_checkout_script.contains("git clone --quiet --depth 1 --no-tags --single-branch")
    );
    assert!(source_checkout_script.contains("source_checkout_status=enabled"));
    assert!(source_checkout_script.contains("mv \"$ROOT_DIR\" \"$BACKUP_DIR\""));
    let workspace_update = include_str!("ui_routes/workspace_update.rs");
    assert!(
        workspace_update.contains("pkill -TERM -f '[t]arget/release/clawd|cargo run -p [c]lawd'")
    );
}

#[test]
fn workspace_update_release_restore_uses_atomic_package_mode() {
    assert_eq!(
        workspace_release_deploy_args(false),
        vec!["./deploy-github-release.sh", "--root", ".", "--no-restart"]
    );
    assert_eq!(
        workspace_release_deploy_args(true),
        vec![
            "./deploy-github-release.sh",
            "--root",
            ".",
            "--no-restart",
            "--package-mode",
        ]
    );
    assert_eq!(
        WorkspaceUpdateMode::ReleaseRestore.as_str(),
        "release_restore"
    );
    let routes = include_str!("ui_routes.rs");
    assert!(routes.contains("/admin/workspace-update/restore-release"));
}

#[test]
fn workspace_update_nginx_scripts_cover_upgrade_disable_and_release_packaging() {
    let deploy_script = include_str!("../../../../deploy-ui-nginx.sh");
    assert!(deploy_script.contains("--upgrade-nginx"));
    assert!(deploy_script.contains("brew upgrade nginx"));
    assert!(deploy_script.contains("apt-get install -y nginx"));
    assert!(deploy_script.contains("apk add --upgrade nginx"));
    assert!(deploy_script.contains("add_header X-Frame-Options \"DENY\" always;"));
    assert!(deploy_script.contains("add_header Content-Security-Policy"));

    let build_script = include_str!("../../../../build-all.sh");
    assert!(build_script.contains("preserve-nginx"));
    assert!(build_script.contains("Preserving nginx as requested"));
    assert!(build_script.contains("APP_PRESERVE_NGINX"));

    let disable_script = include_str!("../../../../scripts/disable-nginx-web.sh");
    assert!(disable_script.contains("brew services stop nginx"));
    assert!(disable_script.contains("systemctl disable --now nginx"));
    assert!(disable_script.contains("Agent Runtime UI"));
    assert!(disable_script.contains("*/\"$APP_DATA_NAMESPACE\"|*/nginx-ui"));
    assert!(disable_script.contains("Refusing to delete non-dedicated UI root"));

    let package_script = include_str!("../../../../package-release.sh");
    assert!(package_script.contains("copy_if_exists \"deploy-ui-nginx.sh\""));
    assert!(package_script.contains("copy_if_exists \"scripts\""));
}

#[test]
fn workspace_update_systemd_unit_name_accepts_only_machine_tokens() {
    assert!(is_safe_systemd_unit_name("agent-runtime.service"));
    assert!(is_safe_systemd_unit_name("agent-runtime-worker@1.service"));
    assert!(!is_safe_systemd_unit_name(""));
    assert!(!is_safe_systemd_unit_name("agent-runtime.service; reboot"));
    assert!(!is_safe_systemd_unit_name("agent-runtime service"));
}
