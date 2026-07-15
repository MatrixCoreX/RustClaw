use super::{filter_catalog_response, model_catalog_text_lines};

#[test]
fn models_catalog_filter_and_text_lines_use_machine_tokens() {
    let body = serde_json::json!({
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
    });

    let filtered = filter_catalog_response(body, Some("minimax"));
    let entries = filtered
        .pointer("/data/entries")
        .and_then(serde_json::Value::as_array)
        .expect("entries");
    assert_eq!(entries.len(), 1);

    let lines = model_catalog_text_lines(&filtered);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("model_catalog_entry provider=minimax model=MiniMax-M3"));
    assert!(lines[0].contains("active=1"));
    assert!(lines[0].contains("credential_state=configured_inline"));
    assert!(lines[0].contains("image_input=1"));
    assert!(lines[0].contains("audio_input=0"));
    assert!(lines[0].contains("music_generation=1"));
}
