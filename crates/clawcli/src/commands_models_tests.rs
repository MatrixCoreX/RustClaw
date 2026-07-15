use super::{
    filter_catalog_response, model_catalog_text_lines, model_readiness_json,
    model_readiness_text_lines,
};

fn model_catalog_fixture() -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "data": {
            "schema_version": 1,
            "selected_provider": "minimax",
            "selected_model": "MiniMax-M3",
            "entries": [
                {
                    "provider": "minimax",
                    "model": "MiniMax-M3",
                    "active_text_provider": true,
                    "api_style": "openai_compatible",
                    "base_url_kind": "minimax_official_openai_compat",
                    "credential_state": "configured_inline",
                    "context_window_tokens": 1000000,
                    "input_modalities": ["text", "image", "video"],
                    "output_modalities": ["text"],
                    "supports_text": true,
                    "supports_image_input": true,
                    "supports_video_input": true,
                    "supports_audio_input": false,
                    "supports_image_understanding": true,
                    "supports_audio_transcription": true,
                    "supports_image_generation": true,
                    "supports_audio_generation": true,
                    "supports_video_generation": true,
                    "supports_music_generation": true,
                    "async_required": true,
                    "dry_run_supported": true
                },
                {
                    "provider": "qwen",
                    "model": "qwen-max-latest",
                    "active_text_provider": false,
                    "api_style": "openai_compatible",
                    "base_url_kind": "qwen_dashscope_openai_compat",
                    "credential_state": "missing",
                    "context_window_tokens": null,
                    "input_modalities": ["text"],
                    "output_modalities": ["text"],
                    "supports_text": true,
                    "supports_image_input": false,
                    "supports_video_input": false,
                    "supports_audio_input": false,
                    "supports_image_understanding": true,
                    "supports_audio_transcription": true,
                    "supports_image_generation": true,
                    "supports_audio_generation": true,
                    "supports_video_generation": false,
                    "supports_music_generation": false,
                    "async_required": true,
                    "dry_run_supported": true
                }
            ]
        }
    })
}

#[test]
fn models_catalog_filter_and_text_lines_use_machine_tokens() {
    let body = model_catalog_fixture();

    let filtered = filter_catalog_response(body, Some("minimax"));
    let entries = filtered
        .pointer("/data/entries")
        .and_then(serde_json::Value::as_array)
        .expect("entries");
    assert_eq!(entries.len(), 1);

    let lines = model_catalog_text_lines(&filtered);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("model_catalog_summary"));
    assert!(lines[0].contains("schema_version=1"));
    assert!(lines[0].contains("selected_provider=minimax"));
    assert!(lines[0].contains("selected_model=MiniMax-M3"));
    assert!(lines[0].contains("entry_count=1"));
    assert!(lines[1].contains("model_catalog_entry provider=minimax model=MiniMax-M3"));
    assert!(lines[1].contains("active=1"));
    assert!(lines[1].contains("credential_state=configured_inline"));
    assert!(lines[1].contains("context_window_tokens=1000000"));
    assert!(lines[1].contains("input_modalities=text,image,video"));
    assert!(lines[1].contains("output_modalities=text"));
    assert!(lines[1].contains("image_input=1"));
    assert!(lines[1].contains("audio_input=0"));
    assert!(lines[1].contains("video_generation=1"));
    assert!(lines[1].contains("music_generation=1"));
    assert!(lines[1].contains("async_required=1"));
    assert!(lines[1].contains("dry_run=1"));
}

#[test]
fn models_readiness_text_and_json_use_selected_catalog_entry() {
    let body = model_catalog_fixture();

    let readiness = model_readiness_json(&body);
    assert_eq!(readiness["schema_version"], 1);
    assert_eq!(readiness["selected_provider"], "minimax");
    assert_eq!(readiness["selected_model"], "MiniMax-M3");
    assert_eq!(readiness["selected_entry_status"], "found");
    assert_eq!(readiness["entry_count"], 2);
    assert_eq!(readiness["matched_entry_count"], 1);
    assert_eq!(readiness["credential_state"], "configured_inline");
    assert_eq!(readiness["ready"], true);
    assert_eq!(readiness["text_generation"], true);
    assert_eq!(readiness["image_input"], true);
    assert_eq!(readiness["image_understanding"], true);
    assert_eq!(readiness["image_generation"], true);
    assert_eq!(readiness["audio_input"], false);
    assert_eq!(readiness["audio_transcription"], true);
    assert_eq!(readiness["audio_generation"], true);
    assert_eq!(readiness["video_input"], true);
    assert_eq!(readiness["video_generation"], true);
    assert_eq!(readiness["music_generation"], true);
    assert_eq!(readiness["async_required"], true);
    assert_eq!(readiness["dry_run"], true);

    let lines = model_readiness_text_lines(&body);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("model_readiness_summary"));
    assert!(lines[0].contains("schema_version=1"));
    assert!(lines[0].contains("selected_provider=minimax"));
    assert!(lines[0].contains("selected_model=MiniMax-M3"));
    assert!(lines[0].contains("selected_entry_status=found"));
    assert!(lines[0].contains("entry_count=2"));
    assert!(lines[0].contains("matched_entry_count=1"));
    assert!(lines[0].contains("credential_state=configured_inline"));
    assert!(lines[0].contains("ready=1"));
    assert!(lines[0].contains("text_generation=1"));
    assert!(lines[0].contains("image_input=1"));
    assert!(lines[0].contains("image_understanding=1"));
    assert!(lines[0].contains("image_generation=1"));
    assert!(lines[0].contains("audio_input=0"));
    assert!(lines[0].contains("audio_transcription=1"));
    assert!(lines[0].contains("audio_generation=1"));
    assert!(lines[0].contains("video_input=1"));
    assert!(lines[0].contains("video_generation=1"));
    assert!(lines[0].contains("music_generation=1"));
    assert!(lines[0].contains("async_required=1"));
    assert!(lines[0].contains("dry_run=1"));
}

#[test]
fn models_readiness_marks_missing_selected_entry_not_ready() {
    let mut body = model_catalog_fixture();
    body["data"]["selected_model"] = serde_json::json!("missing-model");

    let readiness = model_readiness_json(&body);
    assert_eq!(readiness["selected_entry_status"], "missing");
    assert_eq!(readiness["matched_entry_count"], 0);
    assert_eq!(readiness["credential_state"], "null");
    assert_eq!(readiness["ready"], false);
    assert_eq!(readiness["text_generation"], false);

    let lines = model_readiness_text_lines(&body);
    assert!(lines[0].contains("selected_entry_status=missing"));
    assert!(lines[0].contains("matched_entry_count=0"));
    assert!(lines[0].contains("credential_state=null"));
    assert!(lines[0].contains("ready=0"));
}
