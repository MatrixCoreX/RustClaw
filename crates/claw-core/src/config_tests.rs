use super::AppConfig;
use std::fs;

fn unique_temp_config_dir(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "rustclaw-claw-core-config-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    dir
}

#[test]
fn app_config_load_allows_missing_telegram_split_config() {
    let dir = unique_temp_config_dir("missing-telegram");
    fs::create_dir_all(&dir).expect("create temp config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
listen = "127.0.0.1:0"
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]
"#,
    )
    .expect("write temp config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("config without telegram split file should load");

    assert!(cfg.telegram.bot_token.is_empty());
    assert_eq!(cfg.telegram.agent_id, "main");
    assert!(cfg.telegram_runtime_bots().is_empty());

    fs::remove_dir_all(dir).expect("remove temp config dir");
}
