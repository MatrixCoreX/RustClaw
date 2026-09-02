use super::*;
use serde_json::json;

#[test]
fn whatsapp_delivery_keeps_web_and_cloud_adapters_distinct() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_cloud_enabled = true;

    assert_eq!(
        resolve_whatsapp_delivery_route(&state, &json!({"adapter": "whatsapp_web"})),
        crate::WhatsappDeliveryRoute::WebBridge
    );
    assert_eq!(
        resolve_whatsapp_delivery_route(&state, &json!({"adapter": "whatsapp_cloud"})),
        crate::WhatsappDeliveryRoute::Cloud
    );
}

#[test]
fn whatsapp_delivery_falls_back_to_the_only_enabled_adapter() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_cloud_enabled = false;

    assert_eq!(
        resolve_whatsapp_delivery_route(&state, &json!({})),
        crate::WhatsappDeliveryRoute::WebBridge
    );
}

#[test]
fn wechat_delivery_uses_raw_reply_target_not_scoped_conversation_id() {
    let payload = json!({
        "context_token": "stale-token",
        "external_chat_id": "wechat-scope-v1:opaque",
        "channel_ingress": {
            "context_token": "pinned-token",
            "reply_target": {"kind": "user", "external_id": "raw-peer"}
        }
    });

    assert_eq!(
        ingress_reply_target_from_payload(&payload).as_deref(),
        Some("raw-peer")
    );
    assert_eq!(
        ingress_context_token_from_payload(&payload),
        Some("pinned-token")
    );
}

#[test]
fn scheduled_wechat_delivery_resolves_latest_inbound_context_after_chat_rotation() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_key = "rk-wechat-schedule-context";
    state.seed_test_auth_identity(user_key, "user");
    let principal_id = {
        let db = state.core.db.get().expect("db");
        crate::repo::auth::principal_id_for_user_key(&db, user_key)
            .expect("resolve principal")
            .expect("principal")
    };
    let payload = json!({
        "text": "current inbound request",
        "channel_ingress": {
            "context_token": "fresh-context-token",
            "reply_target": {"kind": "user", "external_id": "wechat-user"}
        }
    });
    {
        let db = state.core.db.get().expect("db");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, principal_id, channel,
                external_user_id, external_chat_id, kind, payload_json,
                status, created_at, updated_at
             ) VALUES (
                'current-inbound', 99, 200, ?1, ?2, 'wechat',
                'wechat-user', 'wechat-scope-v1:current', 'ask', ?3,
                'succeeded', '200', '200'
             )",
            rusqlite::params![user_key, principal_id, payload.to_string()],
        )
        .expect("insert current inbound task");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, principal_id, channel,
                external_user_id, external_chat_id, kind, payload_json,
                status, created_at, updated_at
             ) VALUES (
                'scheduled-child', 99, 100, ?1, ?2, 'wechat',
                'wechat-user', 'wechat-user', 'ask', ?3,
                'succeeded', '300', '300'
             )",
            rusqlite::params![
                user_key,
                principal_id,
                json!({
                    "schedule_triggered": true,
                    "context_token": "scheduled-stale-token"
                })
                .to_string()
            ],
        )
        .expect("insert scheduled child task");
    }
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "scheduled-child".to_string(),
        user_id: 99,
        chat_id: 100,
        user_key: Some(user_key.to_string()),
        channel: "wechat".to_string(),
        external_user_id: Some("wechat-user".to_string()),
        external_chat_id: Some("wechat-user".to_string()),
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    assert_eq!(
        latest_wechat_inbound_context_token(&state, &task, "wechat-user")
            .expect("resolve latest inbound context")
            .as_deref(),
        Some("fresh-context-token")
    );
    assert_eq!(
        latest_wechat_inbound_context_token(&state, &task, "other-user")
            .expect("do not cross users"),
        None
    );
}
