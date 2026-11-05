use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use tower::ServiceExt;

async fn request(fixture: &Fixture, key: &str, uri: &str) -> (StatusCode, Value) {
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(fixture.state.clone());
    let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    *request.headers_mut() = headers(key);
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn model_catalog_real_http_routes_preserve_success_and_failure_contracts() {
    let fixture = Fixture::new();
    for key in [OWNER, ADMIN, OTHER, "", "fixture-catalog-unknown"] {
        assert_eq!(
            request(&fixture, key, "/v1/models/catalog").await,
            fixture.catalog(key).await
        );
        assert_eq!(
            request(
                &fixture,
                key,
                &format!("/v1/debug/tasks/{TASK}?teaching=true")
            )
            .await,
            fixture.debug(key, TASK, true).await
        );
    }
    fixture.write("configs/image.toml", &format!("api_key = {SECRET}\n"));
    let failed = request(&fixture, OWNER, "/v1/models/catalog").await;
    assert_error(
        &failed,
        StatusCode::INTERNAL_SERVER_ERROR,
        "model_catalog_unavailable",
    );
    fixture.assert_private(&failed.1);
    let trace = request(
        &fixture,
        OWNER,
        &format!("/v1/debug/tasks/{TASK}?teaching=true"),
    )
    .await;
    assert_eq!(trace.0, StatusCode::OK);
    assert_eq!(
        trace.1["data"]["model_catalog_trace"]["failure_code"],
        "model_catalog_parse_failed"
    );
    fixture.assert_private(&trace.1);
    let no_opt_in = request(&fixture, OWNER, &format!("/v1/debug/tasks/{TASK}")).await;
    assert_error(
        &no_opt_in,
        StatusCode::BAD_REQUEST,
        "teaching_trace_opt_in_required",
    );
}

#[tokio::test]
async fn model_catalog_debug_http_preserves_pagination_numbering_and_redaction() {
    let fixture = Fixture::new();
    let entries = [3, 1, 2]
        .map(|n| {
            json!({
                "ts": n, "task_id":TASK, "call_id":format!("call-{n}"),
                "provider":"fixture", "model":"fixture-model", "status":"ok", "mode":"verbose",
                "request_payload":{"headers":{"authorization":SECRET}, "messages":[]},
                "response":format!("fixture-response-{n}"),
            })
            .to_string()
        })
        .join("\n");
    fixture.write("logs/model_io.log", &entries);
    let uri = format!("/v1/debug/tasks/{TASK}?teaching=true&limit=2&offset=1");
    let response = request(&fixture, OWNER, &uri).await;
    assert_eq!(response.0, StatusCode::OK);
    let data = &response.1["data"];
    assert_eq!(data["trace_schema_version"], 2);
    assert_eq!(data["call_count"], 3);
    assert_eq!(
        data["pagination"],
        json!({"limit":2,"offset":1,"total":3,"has_more":false})
    );
    for (index, expected_call) in [1, 2].into_iter().enumerate() {
        assert_eq!(
            data["entries"][index]["call_id"],
            format!("call-{expected_call}")
        );
        assert_eq!(data["calls"][index]["call_index"], index + 2);
        assert_eq!(
            data["calls"][index]["call_id"],
            format!("call-{expected_call}")
        );
    }
    assert_eq!(data["redaction"]["applied"], true);
    assert_eq!(data["redaction"]["policy"], "teaching_trace_v1");
    fixture.assert_private(&response.1);
    let expected_entries = read_task_debug_entries_page(&fixture.state, TASK, 2, 1).unwrap();
    assert_eq!(
        data["model_catalog_trace"],
        build_model_catalog_trace_for_debug(&fixture.state, &expected_entries.entries)
    );
}
