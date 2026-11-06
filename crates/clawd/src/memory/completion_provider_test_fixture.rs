//! A private, reloadable provider configuration for the actual recovery worker.
//! Reopening a worker must not depend on its old in-memory model-selection cache.
use std::io::Write;
use std::path::Path;

use serde_json::json;

pub(super) fn install(state: &mut crate::AppState, runtime: &crate::LlmProviderRuntime) {
    let database: String = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get(0),
        )
        .expect("owned file-backed recovery database");
    assert!(!database.is_empty());
    let path = Path::new(&database).parent().unwrap().join("provider.toml");
    let config = toml::Value::try_from(json!({
        "server":{"request_timeout_seconds":30},
        "database":{"sqlite_path":database,"busy_timeout_ms":50},
        "worker":{},
        "llm":{"providers":[{
            "name":runtime.config.name,"type":runtime.config.provider_type,
            "base_url":runtime.config.base_url,"api_key":runtime.config.api_key,
            "model":runtime.config.model,"supports_tools":runtime.config.supports_tools,
            "input_modalities":runtime.config.input_modalities,
            "priority":runtime.config.priority,"timeout_seconds":runtime.config.timeout_seconds,
            "max_concurrency":runtime.config.max_concurrency,"params":{"stream":false}
        }]}
    }))
    .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("exclusive config within owned fixture root");
    file.write_all(toml::to_string(&config).unwrap().as_bytes())
        .unwrap();
    drop(file);
    state.reload_ctx.config_path_for_reload = path.to_string_lossy().into_owned();
    let loaded = claw_core::config::AppConfig::load(&path.to_string_lossy())
        .expect("normal provider configuration reload");
    let providers = crate::llm_gateway::build_providers_for_selection(
        &loaded,
        Some(&runtime.config.provider_type),
        Some(&runtime.config.model),
    );
    assert_eq!(
        providers.len(),
        1,
        "one isolated loopback provider after reload"
    );
    assert_eq!(providers[0].config.name, runtime.config.name);
    assert_eq!(providers[0].config.base_url, runtime.config.base_url);
    assert_eq!(providers[0].config.model, runtime.config.model);
    assert_eq!(
        providers[0].model_capabilities(),
        runtime.model_capabilities()
    );
}
