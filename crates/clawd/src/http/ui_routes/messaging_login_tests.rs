use super::*;

#[test]
fn wechat_login_session_is_owned_by_one_principal() {
    let mut store = WechatUiLoginSessionStore::default();
    let token = store.reserve("principal-a", 100).expect("reserve session");
    assert_eq!(
        store
            .reserve("principal-a", 101)
            .expect("reuse own session"),
        token
    );
    assert_eq!(
        store.reserve("principal-b", 101),
        Err("wechat.login_session_in_use")
    );

    store
        .attach_provider_session(&token, "principal-a", "provider-primary".to_string(), 102)
        .expect("attach provider session");
    assert_eq!(
        store.resolve(&token, "principal-b", 103).unwrap_err(),
        "wechat.login_session_owner_mismatch"
    );
    assert_eq!(
        store
            .resolve(&token, "principal-a", 103)
            .expect("resolve owner session")
            .provider_session_key
            .as_deref(),
        Some("provider-primary")
    );
}

#[test]
fn expired_wechat_login_session_releases_the_channel() {
    let mut store = WechatUiLoginSessionStore::default();
    let token = store.reserve("principal-a", 100).expect("reserve session");
    let after_expiry = 100 + WECHAT_UI_LOGIN_SESSION_TTL_SECONDS + 1;

    assert_eq!(
        store
            .resolve(&token, "principal-a", after_expiry)
            .unwrap_err(),
        "wechat.login_session_expired"
    );
    assert!(store.reserve("principal-b", after_expiry).is_ok());
}

#[test]
fn wechat_status_projection_requires_current_user_binding() {
    let connected = json!({
        "connected": true,
        "qr_ready": false,
        "status": "connected",
        "user_id": "wx-user-a",
    });

    let current = project_wechat_login_status(connected.clone(), true, None)
        .expect("project current user status");
    assert_eq!(current.get("provider_connected"), Some(&Value::Bool(true)));
    assert_eq!(current.get("current_user_bound"), Some(&Value::Bool(true)));
    assert_eq!(current.get("connected"), Some(&Value::Bool(true)));
    assert!(current.get("user_id").is_none());

    let other =
        project_wechat_login_status(connected, false, None).expect("project other user status");
    assert_eq!(other.get("provider_connected"), Some(&Value::Bool(true)));
    assert_eq!(other.get("current_user_bound"), Some(&Value::Bool(false)));
    assert_eq!(other.get("connected"), Some(&Value::Bool(false)));
    assert_eq!(
        other.get("binding_status").and_then(Value::as_str),
        Some("connected_unbound")
    );
}

#[test]
fn wechat_status_hides_qr_from_non_owner() {
    let status = json!({
        "connected": false,
        "qr_ready": true,
        "session_key": "provider-primary",
        "qrcode_url": "data:image/svg+xml;base64,secret",
        "message": "provider message",
        "user_id": "wx-user-a",
    });

    let hidden =
        project_wechat_login_status(status.clone(), false, None).expect("project hidden status");
    assert_eq!(hidden.get("qr_ready"), Some(&Value::Bool(false)));
    assert!(hidden.get("session_key").is_none());
    assert!(hidden.get("qrcode_url").is_none());
    assert!(hidden.get("message").is_none());

    let visible = project_wechat_login_status(status, false, Some("client-token"))
        .expect("project owner status");
    assert_eq!(
        visible.get("session_key").and_then(Value::as_str),
        Some("client-token")
    );
    assert!(visible.get("qrcode_url").is_some());
}
