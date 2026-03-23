use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn parse_toml(path: &Path) -> toml::Value {
    toml::from_str(&fs::read_to_string(path).expect("read config")).expect("parse toml")
}

fn minimax_models(value: &toml::Value) -> Vec<String> {
    value["llm"]["minimax"]["models"]
        .as_array()
        .expect("minimax models")
        .iter()
        .filter_map(|item| item.as_str())
        .map(str::to_string)
        .collect()
}

fn minimax_default_model(value: &toml::Value) -> String {
    value["llm"]["minimax"]["model"]
        .as_str()
        .expect("minimax default model")
        .to_string()
}

#[test]
fn minimax_templates_allow_the_repo_default_model() {
    let root = workspace_root();
    let root_config = parse_toml(&root.join("configs/config.toml"));
    let docker_config = parse_toml(&root.join("docker/config/config.toml"));

    let selected_model = root_config["llm"]["selected_model"]
        .as_str()
        .expect("root selected model");
    let root_models = minimax_models(&root_config);
    let docker_models = minimax_models(&docker_config);

    assert!(
        root_models.iter().any(|model| model == selected_model),
        "root minimax models should include selected model {selected_model}, got {root_models:?}"
    );
    assert!(
        docker_models.iter().any(|model| model == selected_model),
        "docker minimax models should include selected model {selected_model}, got {docker_models:?}"
    );
    assert_eq!(
        minimax_default_model(&root_config),
        minimax_default_model(&docker_config),
        "root and docker minimax defaults should stay aligned",
    );
}
