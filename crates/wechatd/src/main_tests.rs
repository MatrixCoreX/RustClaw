use super::{
    build_login_status_response, extract_bind_key_candidate, extract_text_message,
    is_unbound_allowed_command, qr_render_content, qr_svg_data_url, task_success_messages,
    wechat_media_agent_context, wechat_runtime_status_file_path, wechat_t,
    workspace_root_from_config_path, ActiveLogin, MessageItem, QRCodeResponse, TaskQueryResponse,
    TaskStatus, TextItem, VoiceItem, WechatRuntimeStatus, WechatSection, WeixinMessage,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[test]
fn workspace_root_comes_from_channel_config_path() {
    let root = workspace_root_from_config_path("/tmp/demo/configs/channels/wechat.toml");
    assert_eq!(root, PathBuf::from("/tmp/demo"));
}

#[test]
fn runtime_status_path_is_under_run_directory() {
    let path = wechat_runtime_status_file_path(Path::new("/tmp/demo"));
    assert_eq!(
        path,
        PathBuf::from("/tmp/demo/run/wechatd-status/primary.json")
    );
}

#[test]
fn qr_svg_data_url_returns_svg_data_uri() {
    let data_url = qr_svg_data_url("https://example.com/qr-login").expect("qr svg");
    assert!(data_url.starts_with("data:image/svg+xml;base64,"));
    assert!(data_url.len() > "data:image/svg+xml;base64,".len());
}

#[test]
fn qr_render_content_prefers_img_content() {
    let response = QRCodeResponse {
        qrcode: "909101143a13a8526f377cf9f2655903".to_string(),
        qrcode_img_content: "https://example.com/wechat-login".to_string(),
    };

    assert_eq!(
        qr_render_content(&response),
        "https://example.com/wechat-login"
    );
}

#[test]
fn qr_render_content_falls_back_to_qrcode_id() {
    let response = QRCodeResponse {
        qrcode: "909101143a13a8526f377cf9f2655903".to_string(),
        qrcode_img_content: "   ".to_string(),
    };

    assert_eq!(
        qr_render_content(&response),
        "909101143a13a8526f377cf9f2655903"
    );
}

#[test]
fn login_status_response_includes_session_key_for_active_qr() {
    let status = WechatRuntimeStatus {
        healthy: true,
        status: "qr_ready".to_string(),
        last_event_ts: Some(123),
        last_peer: None,
        last_error: None,
        account_label: Some("primary".to_string()),
    };
    let active = ActiveLogin {
        session_key: "primary".to_string(),
        qrcode: "qr-id".to_string(),
        qrcode_url: "data:image/svg+xml;base64,abc".to_string(),
        started_at_ms: 100,
        status: "wait".to_string(),
        message: "二维码已生成".to_string(),
    };

    let response = build_login_status_response(&status, Some(&active));

    assert_eq!(response.session_key.as_deref(), Some("primary"));
    assert_eq!(response.qr_status.as_deref(), Some("wait"));
    assert_eq!(
        response.qrcode_url.as_deref(),
        Some("data:image/svg+xml;base64,abc")
    );
}

#[test]
fn login_status_response_omits_session_key_without_active_qr() {
    let status = WechatRuntimeStatus {
        healthy: true,
        status: "connected".to_string(),
        last_event_ts: Some(123),
        last_peer: None,
        last_error: None,
        account_label: Some("bot-1".to_string()),
    };

    let response = build_login_status_response(&status, None);

    assert!(response.session_key.is_none());
    assert_eq!(response.connected, true);
    assert_eq!(response.qr_ready, false);
}

#[test]
fn extract_text_message_prefers_text_items() {
    let msg = WeixinMessage {
        from_user_id: Some("u1".to_string()),
        _to_user_id: None,
        create_time_ms: None,
        item_list: Some(vec![MessageItem {
            r#type: Some(1),
            ref_msg: None,
            text_item: Some(TextItem {
                text: Some("hello".to_string()),
            }),
            voice_item: None,
            image_item: None,
            video_item: None,
            file_item: None,
        }]),
        context_token: Some("ctx".to_string()),
    };
    assert_eq!(extract_text_message(&msg).as_deref(), Some("hello"));
}

#[test]
fn extract_text_message_falls_back_to_voice_transcript() {
    let msg = WeixinMessage {
        from_user_id: Some("u1".to_string()),
        _to_user_id: None,
        create_time_ms: None,
        item_list: Some(vec![MessageItem {
            r#type: Some(3),
            ref_msg: None,
            text_item: None,
            voice_item: Some(VoiceItem {
                text: Some("voice text".to_string()),
                media: None,
            }),
            image_item: None,
            video_item: None,
            file_item: None,
        }]),
        context_token: None,
    };
    assert_eq!(extract_text_message(&msg).as_deref(), Some("voice text"));
}

#[test]
fn unbound_plain_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command("hello"));
    assert_eq!(extract_bind_key_candidate("hello", false), None);
}

#[test]
fn unbound_key_command_keeps_binding_flow_available() {
    assert_eq!(
        extract_bind_key_candidate("/key rk_live_123", false).as_deref(),
        Some("rk_live_123")
    );
}

#[test]
fn unbound_help_and_start_are_allowed() {
    assert!(is_unbound_allowed_command("/start"));
    assert!(is_unbound_allowed_command("/help"));
    assert!(!is_unbound_allowed_command("/start/docs"));
    assert!(!is_unbound_allowed_command("/help.md"));
}

#[test]
fn waiting_key_state_accepts_plain_key_reply() {
    assert_eq!(
        extract_bind_key_candidate("rk_live_abc", true).as_deref(),
        Some("rk_live_abc")
    );
}

#[test]
fn waiting_key_state_rejects_non_binding_commands() {
    assert_eq!(
        extract_bind_key_candidate("/run image_vision {}", true),
        None
    );
    assert_eq!(extract_bind_key_candidate("/crypto btc", true), None);
}

#[test]
fn unbound_media_like_empty_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command(""));
    assert_eq!(extract_bind_key_candidate("", false), None);
}

fn test_wechat_section(language: &str, i18n_path: String) -> WechatSection {
    WechatSection {
        enabled: true,
        listen: "127.0.0.1:0".to_string(),
        clawd_base_url: "http://127.0.0.1:8787".to_string(),
        api_base_url: "https://ilinkai.weixin.qq.com".to_string(),
        language: language.to_string(),
        i18n_path,
        bot_token: String::new(),
        wechat_uin_base64: String::new(),
        request_timeout_seconds: 30,
        task_delivery_timeout_seconds: 600,
        longpoll_timeout_ms: 35_000,
        text_chunk_chars: 1200,
        sk_route_tag: String::new(),
        typing_refresh_interval_secs: 5,
        cdn_base_url: "https://novac2c.cdn.weixin.qq.com/c2c".to_string(),
        image_inbox_dir: "data/wechatd/image".to_string(),
        video_inbox_dir: "data/wechatd/video".to_string(),
        audio_inbox_dir: "data/wechatd/audio".to_string(),
        file_inbox_dir: "data/wechatd/file".to_string(),
    }
}

#[test]
fn wechat_i18n_binding_keys_are_locale_specific_with_key_fallback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let zh = test_wechat_section(
        "zh-CN",
        root.join("configs/i18n/wechatd.zh-CN.toml")
            .to_string_lossy()
            .to_string(),
    );
    let en = test_wechat_section(
        "en-US",
        root.join("configs/i18n/wechatd.en-US.toml")
            .to_string_lossy()
            .to_string(),
    );
    let missing = test_wechat_section("missing", "/tmp/rustclaw-no-such-i18n.toml".to_string());

    assert!(wechat_t(&zh, "wechat.msg.bind_success").contains("绑定成功"));
    assert!(!wechat_t(&zh, "wechat.msg.bind_key_required_for_chat").contains("Please send"));
    assert!(wechat_t(&en, "wechat.msg.bind_success").contains("Key bound"));
    assert!(!wechat_t(&en, "wechat.msg.bind_key_required_for_chat").contains("请先"));
    assert_eq!(
        wechat_t(&missing, "wechat.msg.bind_success"),
        "wechat.msg.bind_success"
    );
}

#[test]
fn wechat_task_success_fallback_uses_i18n() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let en = test_wechat_section(
        "en-US",
        root.join("configs/i18n/wechatd.en-US.toml")
            .to_string_lossy()
            .to_string(),
    );
    let task = TaskQueryResponse {
        task_id: Default::default(),
        status: TaskStatus::Succeeded,
        execution_state: None,
        result_json: None,
        error_text: None,
        lifecycle: None,
    };

    assert_eq!(task_success_messages(&task, &en), vec!["Done.".to_string()]);
}

#[test]
fn wechat_media_agent_context_uses_machine_fields() {
    let text = wechat_media_agent_context(
        "file",
        "data/wechatd/file/user/123_report.pdf",
        Some("report.pdf"),
    );
    let value: Value = serde_json::from_str(&text).expect("media context json");
    assert_eq!(value["event_type"], "channel_media_saved");
    assert_eq!(value["channel"], "wechat");
    assert_eq!(value["media_kind"], "file");
    assert_eq!(
        value["workspace_relative_path"],
        "data/wechatd/file/user/123_report.pdf"
    );
    assert_eq!(value["locator"]["kind"], "workspace_relative_path");
    assert_eq!(
        value["locator"]["path"],
        "data/wechatd/file/user/123_report.pdf"
    );
    assert_eq!(value["file_name"], "report.pdf");
}
