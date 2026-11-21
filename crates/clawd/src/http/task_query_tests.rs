use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    routing::get,
};
use tower::ServiceExt;
#[tokio::test]
async fn child_query_preserves_exact_id_and_requires_its_owner() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("query-owner", "user");
    state.seed_test_auth_identity("query-other", "user");
    let id = format!(
        "{}:child:review:{}",
        Uuid::new_v4(),
        Uuid::new_v4().simple()
    );
    state.core.db.get().unwrap().execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,channel,kind,payload_json,status,created_at,updated_at) VALUES(?1,1,1,'query-owner','ui','ask','{}','succeeded','1','1')",[&id]).unwrap();
    let app = Router::new()
        .route("/tasks/:id", get(get_task))
        .with_state(state);
    for key in [None, Some("query-other"), Some("query-owner")] {
        let mut request = Request::builder().uri(format!("/tasks/{id}"));
        if let Some(key) = key {
            request = request.header("x-agent-key", key);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if key == Some("query-owner") {
                StatusCode::OK
            } else {
                StatusCode::UNAUTHORIZED
            }
        );
        if key == Some("query-owner") {
            let body: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                    .unwrap();
            assert_eq!(body["data"]["task_id"], id);
        }
    }
}
