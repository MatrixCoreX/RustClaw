use super::{
    build_login_status_response, context_token_store_key, extract_bind_key_candidate,
    extract_text_message, is_unbound_allowed_command, qr_render_content, qr_svg_data_url,
    skill_progress_message, wechat_runtime_status_file_path, wechat_t, wechat_task_terminal_kind,
    workspace_root_from_config_path, ActiveLogin, MessageItem, QRCodeResponse, TaskQueryResponse,
    TaskStatus, TextItem, VoiceItem, WechatRuntimeStatus, WechatSection, WechatTaskTerminalKind,
    WechatTypingHeartbeat, WeixinMessage, TYPING_STATUS_CANCEL, TYPING_STATUS_TYPING,
};
use axum::body::Bytes;
use axum::extract::State as AxumState;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
        seq: None,
        message_id: None,
        from_user_id: Some("u1".to_string()),
        _to_user_id: None,
        create_time_ms: None,
        session_id: None,
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
        seq: None,
        message_id: None,
        from_user_id: Some("u1".to_string()),
        _to_user_id: None,
        create_time_ms: None,
        session_id: None,
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
fn quoted_text_uses_language_neutral_marker() {
    let msg = WeixinMessage {
        seq: None,
        message_id: None,
        from_user_id: Some("u1".to_string()),
        _to_user_id: None,
        create_time_ms: None,
        session_id: None,
        item_list: Some(vec![MessageItem {
            r#type: Some(1),
            ref_msg: Some(super::RefMessage {
                title: Some("previous".to_string()),
                message_item: Some(Box::new(MessageItem {
                    r#type: Some(1),
                    ref_msg: None,
                    text_item: Some(TextItem {
                        text: Some("old body".to_string()),
                    }),
                    voice_item: None,
                    image_item: None,
                    video_item: None,
                    file_item: None,
                })),
            }),
            text_item: Some(TextItem {
                text: Some("new body".to_string()),
            }),
            voice_item: None,
            image_item: None,
            video_item: None,
            file_item: None,
        }]),
        context_token: None,
    };

    assert_eq!(
        extract_text_message(&msg).as_deref(),
        Some("[quote: previous | old body]\nnew body")
    );
}

#[test]
fn inbound_message_identity_prefers_provider_message_id() {
    let msg: WeixinMessage = serde_json::from_value(serde_json::json!({
        "message_id": 9223372036854775807_u64,
        "seq": 17,
        "session_id": "session-a",
        "context_token": "context-a"
    }))
    .expect("parse message");

    assert_eq!(
        super::inbound_provider_message_id(&msg).as_deref(),
        Some("9223372036854775807")
    );
}

#[test]
fn inbound_message_identity_fallback_uses_opaque_transport_fields() {
    let first: WeixinMessage = serde_json::from_value(serde_json::json!({
        "seq": 17,
        "session_id": "session-a",
        "context_token": "context-a"
    }))
    .expect("parse first message");
    let replay: WeixinMessage = serde_json::from_value(serde_json::json!({
        "seq": 17,
        "session_id": "session-a",
        "context_token": "different-context"
    }))
    .expect("parse replay");
    let next: WeixinMessage = serde_json::from_value(serde_json::json!({
        "seq": 18,
        "session_id": "session-a",
        "context_token": "context-a"
    }))
    .expect("parse next message");

    let first_id = super::inbound_provider_message_id(&first).expect("first identity");
    assert_eq!(
        super::inbound_provider_message_id(&replay).as_deref(),
        Some(first_id.as_str())
    );
    assert_ne!(
        super::inbound_provider_message_id(&next).as_deref(),
        Some(first_id.as_str())
    );
    assert!(!first_id.contains("session-a"));
    assert!(!first_id.contains("context-a"));
}

#[test]
fn inbound_message_identity_context_fallback_includes_provider_timestamp() {
    let first: super::WeixinMessage = serde_json::from_value(serde_json::json!({
        "from_user_id": "peer-1",
        "create_time_ms": 1000,
        "context_token": "context-1"
    }))
    .expect("first message");
    let replay: super::WeixinMessage = serde_json::from_value(serde_json::json!({
        "from_user_id": "peer-1",
        "create_time_ms": 1000,
        "context_token": "context-1"
    }))
    .expect("replayed message");
    let next: super::WeixinMessage = serde_json::from_value(serde_json::json!({
        "from_user_id": "peer-1",
        "create_time_ms": 1001,
        "context_token": "context-1"
    }))
    .expect("next message");

    let first_id = super::inbound_provider_message_id(&first).expect("first identity");
    assert_eq!(
        super::inbound_provider_message_id(&replay).as_deref(),
        Some(first_id.as_str())
    );
    assert_ne!(
        super::inbound_provider_message_id(&next).as_deref(),
        Some(first_id.as_str())
    );
}

#[test]
fn inbound_idempotency_key_is_account_scoped() {
    assert_eq!(
        super::wechat_inbound_idempotency_key("account-a", "message-7"),
        "wechat_ilink:9:account-a:message-7"
    );
    assert_ne!(
        super::wechat_inbound_idempotency_key("account-a", "message-7"),
        super::wechat_inbound_idempotency_key("account-b", "message-7")
    );
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
fn wechat_i18n_binding_keys_are_locale_specific_with_safe_fallback() {
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
    let missing = test_wechat_section(
        "missing",
        "/tmp/agent-runtime-no-such-i18n.toml".to_string(),
    );

    assert!(wechat_t(&zh, "wechat.msg.bind_success").contains("绑定成功"));
    assert!(!wechat_t(&zh, "wechat.msg.bind_key_required_for_chat").contains("Please send"));
    assert!(wechat_t(&en, "wechat.msg.bind_success").contains("Key bound"));
    assert!(!wechat_t(&en, "wechat.msg.bind_key_required_for_chat").contains("请先"));
    let fallback = wechat_t(&missing, "wechat.msg.bind_success");
    assert_eq!(
        fallback,
        claw_core::channel_i18n::safe_generic_text_for_locale("en-US")
    );
    assert!(!fallback.contains("wechat.msg.bind_success"));
}

#[test]
fn wechat_media_progress_stays_transport_state_without_canned_replies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let zh = test_wechat_section(
        "zh-CN",
        root.join("configs/i18n/wechatd.zh-CN.toml")
            .to_string_lossy()
            .to_string(),
    );
    let mut task = TaskQueryResponse {
        task_id: Default::default(),
        status: TaskStatus::Running,
        execution_state: None,
        goal: None,
        task_plan: None,
        skill_progress: Some(serde_json::json!({
            "seq": 9,
            "event_type": "skill_progress",
            "payload": {
                "schema_version": 1,
                "source": "skill_progress",
                "data_only": true,
                "frame": {
                    "record_type": "skill_progress",
                    "detail_key": "media_download.precheck.starting",
                    "params": {
                        "step_id": "media_precheck",
                        "step_status": "in_progress",
                        "unsafe_display_text": "must not render"
                    }
                }
            }
        })),
        result_json: None,
        error_text: None,
        lifecycle: None,
    };

    assert!(skill_progress_message(&task, &zh).is_none());

    if let Some(params) = task
        .skill_progress
        .as_mut()
        .and_then(|event| event.pointer_mut("/payload/frame/params"))
        .and_then(Value::as_object_mut)
    {
        params.insert(
            "notification_delivery".to_string(),
            serde_json::json!("runtime"),
        );
    }
    task.task_plan = Some(serde_json::json!({
        "schema_version": 1,
        "source": "task_plan",
        "status": "ok",
        "data_only": true,
        "render_owner": "ui_cli_channel_projection",
        "plan_revision": 1,
        "steps": [{
            "step_id": "media_precheck",
            "title": "检查本次媒体任务",
            "status": "in_progress"
        }]
    }));
    assert!(skill_progress_message(&task, &zh).is_none());
    if let Some(params) = task
        .skill_progress
        .as_mut()
        .and_then(|event| event.pointer_mut("/payload/frame/params"))
        .and_then(Value::as_object_mut)
    {
        params.remove("notification_delivery");
    }

    task.task_plan = Some(serde_json::json!({
        "schema_version": 1,
        "source": "task_plan",
        "status": "ok",
        "data_only": true,
        "render_owner": "ui_cli_channel_projection",
        "plan_revision": 1,
        "steps": [{
            "step_id": "media_precheck",
            "title": "检查本次媒体任务",
            "status": "in_progress"
        }]
    }));
    assert_eq!(
        skill_progress_message(&task, &zh),
        Some((9, "检查本次媒体任务".to_string()))
    );
    task.task_plan = None;

    task.skill_progress
        .as_mut()
        .and_then(|event| event.pointer_mut("/payload/frame/detail_key"))
        .map(|detail_key| *detail_key = serde_json::json!("skill_dispatch.queue.waiting"));
    assert!(skill_progress_message(&task, &zh).is_none());

    task.skill_progress
        .as_mut()
        .and_then(|event| event.pointer_mut("/payload/frame/detail_key"))
        .map(|detail_key| *detail_key = serde_json::json!("skill_dispatch.queue.started"));
    assert!(skill_progress_message(&task, &zh).is_none());

    for detail_key in [
        "media_download.download.starting",
        "media_download.download.completed",
        "media_download.transcribe.extracting_audio",
        "media_download.transcribe.recognizing_speech",
        "media_download.transcribe.completed",
    ] {
        task.skill_progress
            .as_mut()
            .and_then(|event| event.pointer_mut("/payload/frame/detail_key"))
            .map(|value| *value = serde_json::json!(detail_key));
        assert!(
            skill_progress_message(&task, &zh).is_none(),
            "detail_key={detail_key} must not become canned chat text"
        );
    }
}

#[test]
fn task_terminal_mapping_covers_success_failure_cancel_and_timeout() {
    assert_eq!(
        wechat_task_terminal_kind(TaskStatus::Succeeded),
        Some(WechatTaskTerminalKind::Succeeded)
    );
    assert_eq!(
        wechat_task_terminal_kind(TaskStatus::Failed),
        Some(WechatTaskTerminalKind::Failed)
    );
    assert_eq!(
        wechat_task_terminal_kind(TaskStatus::Canceled),
        Some(WechatTaskTerminalKind::Canceled)
    );
    assert_eq!(
        wechat_task_terminal_kind(TaskStatus::Timeout),
        Some(WechatTaskTerminalKind::Timeout)
    );
    assert_eq!(wechat_task_terminal_kind(TaskStatus::Queued), None);
    assert_eq!(wechat_task_terminal_kind(TaskStatus::Running), None);
}

#[test]
fn context_token_cache_key_is_account_channel_peer_scoped() {
    assert_ne!(
        context_token_store_key("account-a", "peer"),
        context_token_store_key("account-b", "peer")
    );
    assert_ne!(
        context_token_store_key("account", "peer-a"),
        context_token_store_key("account", "peer-b")
    );
}

#[derive(Clone, Default)]
struct TypingCapture {
    statuses: Arc<Mutex<Vec<i64>>>,
}

async fn capture_typing(AxumState(capture): AxumState<TypingCapture>, body: Bytes) -> Json<Value> {
    let value: Value = serde_json::from_slice(&body).expect("typing body");
    capture
        .statuses
        .lock()
        .expect("typing statuses")
        .push(value["status"].as_i64().expect("typing status"));
    Json(serde_json::json!({"ret": 0}))
}

#[tokio::test]
async fn typing_heartbeat_finish_waits_for_exactly_one_cancel() {
    let capture = TypingCapture::default();
    let router = Router::new()
        .route("/ilink/bot/sendtyping", post(capture_typing))
        .with_state(capture.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind typing server");
    let base_url = format!("http://{}", listener.local_addr().expect("typing addr"));
    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve typing capture");
    });

    let config = test_wechat_section("en-US", String::new());
    let mut heartbeat = WechatTypingHeartbeat::start(
        reqwest::Client::new(),
        config,
        base_url,
        "test-token".to_string(),
        "peer".to_string(),
        "ticket".to_string(),
        Duration::from_secs(60),
    );
    heartbeat.finish().await;

    let statuses = capture.statuses.lock().expect("typing statuses").clone();
    assert_eq!(statuses.first(), Some(&TYPING_STATUS_TYPING));
    assert_eq!(statuses.last(), Some(&TYPING_STATUS_CANCEL));
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == TYPING_STATUS_CANCEL)
            .count(),
        1
    );
}
