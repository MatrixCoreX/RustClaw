use super::resolve_startup_config_path_from;

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
