//! Ordinary selection, explicit fallback groups and malformed-input controls.
use super::*;

#[tokio::test]
async fn model_catalog_runtime_process_overrides_do_not_replace_explicit_task_selection() {
    let name = "model_catalog_runtime_process_overrides_do_not_replace_explicit_task_selection";
    if isolated(
        "projection",
        name,
        &[
            ("APP_PROVIDER_OVERRIDE", "google"),
            ("APP_MODEL_OVERRIDE", "process-model"),
        ],
    ) {
        return;
    }
    let fixture = Fixture::new();
    configure(&fixture, &(standard("minimax", "", true)
        + "\n[llm.google]\nbase_url='https://fixture.invalid/google'\nmodel='google-default'\n"));
    let config = load_config(&fixture);
    let ordinary = crate::llm_gateway::build_providers(&config);
    assert_eq!(ordinary.len(), 1);
    assert_eq!(ordinary[0].config.name, "vendor-google");
    assert_eq!(ordinary[0].config.model, "process-model");
    let task = crate::llm_gateway::build_providers_for_selection(
        &config,
        Some("minimax"),
        Some("fixture-runtime-model"),
    );
    assert_eq!(task.len(), 1);
    assert_eq!(task[0].config.name, "vendor-minimax");
    assert_eq!(task[0].config.model, "fixture-runtime-model");
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["data"]["selected_provider"], "google");
    assert_eq!(response.1["data"]["selected_model"], "process-model");
    assert!(response.1["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["provider"] == "google"
            && row["model"] == "process-model"
            && row["api_style"] == "google_gemini"
            && row["active_text_provider"] == true));
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_runtime_unknown_process_override_does_not_report_configured_selection_ready()
{
    let name =
        "model_catalog_runtime_unknown_process_override_does_not_report_configured_selection_ready";
    if isolated(
        "projection",
        name,
        &[("APP_PROVIDER_OVERRIDE", "fixture-absent")],
    ) {
        return;
    }
    let fixture = Fixture::new();
    configure(&fixture, &standard("minimax", "", true));
    assert!(crate::llm_gateway::build_providers(&load_config(&fixture)).is_empty());
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["data"]["selected_provider"], "fixture-absent");
    assert!(response.1["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["active_text_provider"] == false));
    let debug = fixture.debug(ADMIN, TASK, true).await;
    assert_eq!(debug.0, StatusCode::OK);
    assert_eq!(
        debug.1["data"]["model_catalog_trace"]["readiness"]["ready"],
        false
    );
    fixture.assert_private(&response.1);
    fixture.assert_private(&debug.1);
}

#[tokio::test]
async fn model_catalog_runtime_explicit_priority_not_inactive_selector_determines_ordinary_selection(
) {
    let fixture = Fixture::new();
    configure(&fixture, &(explicit("openai_compat") + &format!(
        "\n[[llm.providers]]\nname='preferred'\ntype='google_gemini'\nbase_url='https://fixture.invalid/google'\n\
         api_key='{SECRET}'\nmodel='preferred-model'\npriority=0\ntimeout_seconds=40\nmax_concurrency=1\n")));
    let actual = runtimes(&fixture);
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0].config.name, "preferred");
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["data"]["selected_provider"], "preferred");
    assert_eq!(response.1["data"]["selected_model"], "preferred-model");
    let entries = response.1["data"]["entries"].as_array().unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|row| row["active_text_provider"] == true)
            .count(),
        1
    );
    assert!(entries
        .iter()
        .any(|row| row["provider"] == "preferred" && row["api_style"] == "google_gemini"));
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_runtime_malformed_typed_vendor_does_not_manufacture_text_or_inline_readiness(
) {
    let name = "model_catalog_runtime_malformed_typed_vendor_does_not_manufacture_text_or_inline_readiness";
    if isolated("projection", name, &[]) {
        return;
    }
    let fixture = Fixture::new();
    configure(
        &fixture,
        &format!(
            "[llm]\nselected_vendor='minimax'\nselected_model='fixture-runtime-model'\n\
        [llm.minimax]\nmodel='fixture-runtime-model'\napi_key='{SECRET}'\n"
        ),
    );
    assert!(claw_core::config::AppConfig::load(
        fixture.root.join("configs/config.toml").to_str().unwrap()
    )
    .is_err());
    let row = entry(&fixture, "minimax").await;
    assert_eq!(row["supports_text"], false);
    assert_eq!(row["credential_state"], "missing");
}

#[tokio::test]
async fn model_catalog_runtime_explicit_text_does_not_remove_independent_media_or_use_legacy_template_keys(
) {
    let fixture = Fixture::new();
    configure(&fixture, &(explicit("openai_compat") + &format!(
        "\n[llm.minimax]\nbase_url='https://fixture.invalid/text'\nmodel='aaa-media'\napi_key='{SECRET}-legacy'\n")));
    fixture.write("configs/audio.toml", &format!("[audio_transcribe]\ndefault_vendor='minimax'\ndefault_model='aaa-media'\n\
        [audio_transcribe.providers.minimax]\nbase_url='https://fixture.invalid/audio'\napi_key='{SECRET}'\n"));
    assert_eq!(only_runtime(&fixture).config.name, "fixture-explicit");
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    let rows = response.1["data"]["entries"].as_array().unwrap();
    assert!(rows
        .iter()
        .any(|row| row["provider"] == "fixture-explicit" && row["supports_text"] == true));
    let media = rows
        .iter()
        .find(|row| row["provider"] == "minimax")
        .unwrap();
    assert_eq!(media["supports_text"], false);
    assert_eq!(media["supports_audio_transcription"], true);
    assert_eq!(media["credential_state"], "configured_inline");
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_runtime_duplicate_explicit_name_projects_the_actual_first_selected_transport(
) {
    let fixture = Fixture::new();
    configure(&fixture, &(explicit("openai_compat") + &format!(
        "\n[[llm.providers]]\nname='fixture-explicit'\ntype='google_gemini'\nbase_url='https://fixture.invalid/fallback'\n\
         api_key='{SECRET}'\nmodel='second-model'\npriority=2\ntimeout_seconds=99\nmax_concurrency=1\n")));
    let actual = crate::llm_gateway::build_providers_for_selection(
        &load_config(&fixture),
        Some("fixture-explicit"),
        Some("second-model"),
    );
    assert_eq!(actual.len(), 2);
    assert!(actual
        .iter()
        .all(|provider| provider.config.model == "second-model"));
    assert_eq!(actual[0].config.provider_type, "openai_compat");
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    let row = response.1["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["model"] == "second-model")
        .unwrap();
    assert_eq!(row["api_style"], "openai_compatible");
    assert_eq!(row["timeout_seconds"], actual[0].config.timeout_seconds);
    fixture.assert_private(&response.1);
}
