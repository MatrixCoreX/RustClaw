//! Selected main/split path tests; no live LLM calls or persisted task execution.
use super::*;

#[path = "model_catalog_truthfulness_additional_tests.rs"]
mod additional;
#[path = "model_catalog_truthfulness_state_tests.rs"]
mod state_contracts;
#[path = "model_catalog_runtime_descriptor_tests.rs"]
mod runtime_contracts;

fn selected_main(model: &str) -> String {
    format!(
        "[llm]\nselected_vendor='custom'\nselected_model='{model}'\n\
             [llm.custom]\nmodel='{model}'\napi_key=''\nbase_url='https://fixture.invalid/v1'\n"
    )
}

#[tokio::test]
async fn model_catalog_truthfulness_http_and_teaching_follow_the_selected_main_file() {
    let mut fixture = Fixture::new();
    fixture.write(
        "selected/runtime.toml",
        &selected_main("fixture-selected-text"),
    );
    fixture.state.reload_ctx.config_path_for_reload = "selected/runtime.toml".into();
    let before = fs::read(fixture.root.join("configs/config.toml")).unwrap();
    for key in [OWNER, ADMIN] {
        let response = fixture.catalog(key).await;
        assert_eq!(response.0, StatusCode::OK);
        assert_eq!(response.1["data"]["selected_provider"], "custom");
        assert_eq!(
            response.1["data"]["selected_model"],
            "fixture-selected-text"
        );
        fixture.assert_private(&response.1);
        let trace = fixture.debug(key, TASK, true).await;
        assert_eq!(trace.0, StatusCode::OK);
        assert_eq!(
            trace.1["data"]["model_catalog_trace"]["selected_model"],
            "fixture-selected-text"
        );
        fixture.assert_private(&trace.1);
    }
    assert_eq!(
        fs::read(fixture.root.join("configs/config.toml")).unwrap(),
        before
    );
}

#[tokio::test]
async fn model_catalog_truthfulness_external_selected_splits_do_not_read_workspace_defaults() {
    let mut fixture = Fixture::new();
    let selected = Fixture::new();
    selected.write(
        "profile/runtime.toml",
        &selected_main("fixture-external-text"),
    );
    selected.write(
        "profile/image.toml",
        r#"
[image_vision]
default_vendor = "custom"
default_model = "fixture-selected-image"
[image_vision.providers.custom]
model = "fixture-selected-image"
models = ["fixture-selected-image"]
base_url = "https://fixture.invalid/v1"
api_key = ""
"#,
    );
    fixture.write(
        "configs/image.toml",
        r#"
[image_vision]
default_vendor = "custom"
default_model = "fixture-unselected-image"
[image_vision.providers.custom]
model = "fixture-unselected-image"
models = ["fixture-unselected-image"]
base_url = "https://fixture.invalid/v1"
api_key = ""
"#,
    );
    fixture.state.reload_ctx.config_path_for_reload = selected
        .root
        .join("profile/runtime.toml")
        .to_str()
        .unwrap()
        .into();
    let defaults = ["configs/config.toml", "configs/image.toml"]
        .map(|name| fs::read(fixture.root.join(name)).unwrap());
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["data"]["selected_model"],
        "fixture-external-text"
    );
    let entries = response.1["data"]["entries"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry["model"] == "fixture-selected-image"
            && entry["supports_image_understanding"] == true));
    assert!(entries
        .iter()
        .all(|entry| entry["model"] != "fixture-unselected-image"));
    fixture.assert_private(&response.1);
    assert!(!response
        .1
        .to_string()
        .contains(selected.root.to_str().unwrap()));
    assert_eq!(
        ["configs/config.toml", "configs/image.toml"]
            .map(|name| fs::read(fixture.root.join(name)).unwrap()),
        defaults
    );
}
