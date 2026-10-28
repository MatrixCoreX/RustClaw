use super::*;
use crate::task_admin_routes::{
    cancel_one_task, cancel_tasks, ActiveTasksScope, CancelOneTaskRequest, CancelTasksRequest,
};

fn scoped_state() -> crate::AppState {
    let state = state_with_goal_task("scope-anchor", json!({"text": "anchor"}));
    let db = state.core.db.get().unwrap();
    let owner = crate::stable_i64_from_key(USER_KEY);
    for (id, user, chat, channel, key) in [
        ("scope-first", owner, 7, "ui", USER_KEY),
        ("scope-second", owner, 7, "ui", USER_KEY),
        ("scope-other-chat", owner, 8, "ui", USER_KEY),
        ("scope-other-channel", owner, 7, "wechat", USER_KEY),
        ("scope-other-user", 99, 7, "ui", "other-key"),
    ] {
        db.execute(
            "INSERT INTO tasks (task_id,user_id,chat_id,user_key,channel,kind,payload_json,status,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,'ask','{}','queued','2','2')",
            rusqlite::params![id,user,chat,key,channel],
        ).unwrap();
    }
    drop(db);
    state
}

fn session_request() -> ActiveTasksRequest {
    ActiveTasksRequest {
        scope: ActiveTasksScope::CurrentSession,
        user_id: 99,
        chat_id: 999,
        exclude_task_id: Some("scope-anchor".into()),
        limit: None,
        offset: None,
    }
}

fn assert_untouched_outside_session(state: &crate::AppState) {
    for (id, status) in [
        ("scope-anchor", "running"),
        ("scope-other-chat", "queued"),
        ("scope-other-channel", "queued"),
        ("scope-other-user", "queued"),
    ] {
        assert_eq!(
            crate::get_task_admin_target(state, id)
                .unwrap()
                .unwrap()
                .status,
            status
        );
    }
}

#[tokio::test]
async fn session_list_derives_actor_channel_and_chat_from_owned_anchor() {
    for role in ["admin", "user"] {
        let state = scoped_state();
        state
            .core
            .db
            .get()
            .unwrap()
            .execute("UPDATE auth_keys SET role=?1", [role])
            .unwrap();
        let (status, Json(response)) =
            list_active_tasks(State(state), auth_headers(), Json(session_request())).await;
        assert_eq!(status, StatusCode::OK);
        let data = response.data.unwrap();
        assert_eq!(data["count"], 2);
        assert_eq!(data["total"], 2);
        assert_eq!(data["scope"]["kind"], "current_session");
        assert_eq!(data["scope"]["chat_id"], 7);
        assert_eq!(data["scope"]["channel"], "ui");
        let ids = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["task_id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["scope-first", "scope-second"]);
    }
}

#[tokio::test]
async fn session_list_pages_and_indexes_only_the_bound_session() {
    let mut request = session_request();
    request.limit = Some(1);
    request.offset = Some(1);
    let (status, Json(response)) =
        list_active_tasks(State(scoped_state()), auth_headers(), Json(request)).await;
    assert_eq!(status, StatusCode::OK);
    let data = response.data.unwrap();
    assert_eq!(data["total"], 2);
    assert_eq!(data["tasks"][0]["task_id"], "scope-second");
    assert_eq!(data["tasks"][0]["index"], 2);
    assert_eq!(data["has_more"], false);
}

#[tokio::test]
async fn session_anchor_is_required_authenticated_and_actor_owned_even_for_admin() {
    for (anchor, headers, expected) in [
        (None, auth_headers(), StatusCode::BAD_REQUEST),
        (Some("missing"), auth_headers(), StatusCode::NOT_FOUND),
        (
            Some("scope-other-user"),
            auth_headers(),
            StatusCode::FORBIDDEN,
        ),
        (
            Some("scope-anchor"),
            HeaderMap::new(),
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let mut request = session_request();
        request.exclude_task_id = anchor.map(str::to_string);
        let state = scoped_state();
        let (status, _) = list_active_tasks(State(state.clone()), headers, Json(request)).await;
        assert_eq!(status, expected);
        assert_untouched_outside_session(&state);
    }
}

#[tokio::test]
async fn session_cancel_all_cannot_expand_to_other_chats_channels_or_actors() {
    let state = scoped_state();
    let (status, Json(response)) = cancel_tasks(
        State(state.clone()),
        auth_headers(),
        Json(CancelTasksRequest {
            scope: ActiveTasksScope::CurrentSession,
            user_id: 99,
            chat_id: 999,
            exclude_task_id: Some("scope-anchor".into()),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response.data.unwrap()["canceled"], 2);
    assert_untouched_outside_session(&state);
    for id in ["scope-first", "scope-second"] {
        assert_eq!(
            crate::get_task_admin_target(&state, id)
                .unwrap()
                .unwrap()
                .status,
            "canceled"
        );
    }
}

#[tokio::test]
async fn session_cancel_one_fences_expected_id_and_uses_session_index() {
    let state = scoped_state();
    let request = |expected: &str| CancelOneTaskRequest {
        scope: ActiveTasksScope::CurrentSession,
        user_id: 99,
        chat_id: 999,
        index: 1,
        exclude_task_id: Some("scope-anchor".into()),
        expected_task_id: Some(expected.into()),
    };
    let (status, _) = cancel_one_task(
        State(state.clone()),
        auth_headers(),
        Json(request("scope-second")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        crate::get_task_admin_target(&state, "scope-first")
            .unwrap()
            .unwrap()
            .status,
        "queued"
    );
    let (status, Json(response)) = cancel_one_task(
        State(state.clone()),
        auth_headers(),
        Json(request("scope-first")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response.data.unwrap()["task"]["task_id"], "scope-first");
    assert_eq!(
        crate::get_task_admin_target(&state, "scope-second")
            .unwrap()
            .unwrap()
            .status,
        "queued"
    );
    assert_untouched_outside_session(&state);
}

#[tokio::test]
async fn foreign_anchor_cannot_authorize_session_cancellation() {
    let state = scoped_state();
    let (status, _) = cancel_tasks(
        State(state.clone()),
        auth_headers(),
        Json(CancelTasksRequest {
            scope: ActiveTasksScope::CurrentSession,
            user_id: 99,
            chat_id: 7,
            exclude_task_id: Some("scope-other-user".into()),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_untouched_outside_session(&state);
}

#[test]
fn session_scope_rejects_unknown_machine_tokens() {
    let value = json!({"scope":"anywhere","user_id":0,"chat_id":0});
    assert!(serde_json::from_value::<ActiveTasksRequest>(value).is_err());
    let request: ActiveTasksRequest =
        serde_json::from_value(json!({"user_id":0,"chat_id":0})).unwrap();
    assert_eq!(request.scope, ActiveTasksScope::Accessible);
}
