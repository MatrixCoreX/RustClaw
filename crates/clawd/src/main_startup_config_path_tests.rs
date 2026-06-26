use super::{resolve_startup_config_path_from, startup_isolation_cleanup_age_seconds};

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
fn startup_isolation_cleanup_uses_conservative_minimum_age() {
    assert_eq!(startup_isolation_cleanup_age_seconds(60), 6 * 60 * 60);
    assert_eq!(startup_isolation_cleanup_age_seconds(10_000), 40_000);
}
