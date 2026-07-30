use super::{
    resolve_offline_bundled_repair_skill_from, resolve_startup_config_path_from,
    startup_isolation_cleanup_age_seconds, tokio_worker_stack_bytes,
    DEFAULT_TOKIO_WORKER_STACK_BYTES, MAX_TOKIO_WORKER_STACK_BYTES, MIN_TOKIO_WORKER_STACK_BYTES,
};

#[test]
fn prefers_cli_config_path() {
    let resolved = resolve_startup_config_path_from(
        vec!["--config".to_string(), "/tmp/fixture.toml".to_string()],
        Some("/tmp/env.toml".to_string()),
    )
    .expect("resolve config path");
    assert_eq!(resolved, "/tmp/fixture.toml");
}

#[test]
fn falls_back_to_env_config_path() {
    let resolved =
        resolve_startup_config_path_from(Vec::<String>::new(), Some("/tmp/env.toml".to_string()))
            .expect("resolve config path");
    assert_eq!(resolved, "/tmp/env.toml");
}

#[test]
fn resolves_offline_bundled_repair_skill_from_separate_argument() {
    let resolved = resolve_offline_bundled_repair_skill_from(vec![
        "--config".to_string(),
        "/tmp/fixture.toml".to_string(),
        "--repair-bundled-skill".to_string(),
        "media_download".to_string(),
    ])
    .expect("resolve bundled repair skill");
    assert_eq!(resolved.as_deref(), Some("media_download"));
}

#[test]
fn resolves_offline_bundled_repair_skill_from_equals_argument() {
    let resolved = resolve_offline_bundled_repair_skill_from(vec![
        "--repair-bundled-skill=media_download".to_string(),
    ])
    .expect("resolve bundled repair skill");
    assert_eq!(resolved.as_deref(), Some("media_download"));
}

#[test]
fn rejects_missing_or_empty_offline_bundled_repair_skill() {
    assert!(
        resolve_offline_bundled_repair_skill_from(vec!["--repair-bundled-skill".to_string()])
            .is_err()
    );
    assert!(resolve_offline_bundled_repair_skill_from(vec![
        "--repair-bundled-skill=   ".to_string()
    ])
    .is_err());
}

#[test]
fn startup_isolation_cleanup_uses_conservative_minimum_age() {
    assert_eq!(startup_isolation_cleanup_age_seconds(60), 6 * 60 * 60);
    assert_eq!(startup_isolation_cleanup_age_seconds(10_000), 40_000);
}

#[test]
fn tokio_worker_stack_uses_portable_bounded_defaults() {
    assert_eq!(
        tokio_worker_stack_bytes(None),
        DEFAULT_TOKIO_WORKER_STACK_BYTES
    );
    assert_eq!(
        tokio_worker_stack_bytes(Some("1024")),
        MIN_TOKIO_WORKER_STACK_BYTES
    );
    assert_eq!(
        tokio_worker_stack_bytes(Some("134217728")),
        MAX_TOKIO_WORKER_STACK_BYTES
    );
    assert_eq!(tokio_worker_stack_bytes(Some("8388608")), 8 * 1024 * 1024);
    assert_eq!(
        tokio_worker_stack_bytes(Some("invalid")),
        DEFAULT_TOKIO_WORKER_STACK_BYTES
    );
}
