use super::*;
use axum::{http::StatusCode, routing::post, Router};
#[tokio::test]
async fn task_cancel_stops_real_stalled_http_in_both_gateway_paths() {
    for native in [false, true] {
        let reached = Arc::new(tokio::sync::Notify::new());
        let notify = reached.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(move || {
                let notify = notify.clone();
                async move {
                    notify.notify_one();
                    std::future::pending::<StatusCode>().await
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let template = state.core.llm_providers[0].clone();
        let mut config = template.config.clone();
        config.provider_type = "openai_compat".into();
        config.base_url = format!("http://{address}");
        config.timeout_seconds = 60;
        config.params.stream = Some(false);
        let provider = Arc::new(LlmProviderRuntime {
            config,
            pricing: None,
            latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
            client: crate::providers::build_llm_http_client(60).unwrap(),
            semaphore: Arc::new(Semaphore::new(1)),
            breaker: Arc::new(crate::providers::CircuitBreaker::new()),
        });
        state.core.llm_providers = vec![provider.clone()];
        state.seed_ask_task_row("cancel-http", 7, 11, r#"{"text":"fixture input"}"#);
        let task = crate::repo::claim_next_task(&state).unwrap().unwrap();
        state.worker.register_active_task(&task.task_id);
        let current = state.clone();
        let child = task.clone();
        let running = tokio::spawn(async move {
            if native {
                run_native_model_turn_with_fallback(
                    &current,
                    &child,
                    "fixture prompt",
                    "test:cancellation",
                    &claw_core::model_turn::ModelTurnRequest::text("fixture prompt"),
                )
                .await
                .map(|_| ())
            } else {
                run_with_fallback_with_prompt_source(
                    &current,
                    &child,
                    "fixture prompt",
                    "test:cancellation",
                )
                .await
                .map(|_| ())
            }
        });
        tokio::time::timeout(Duration::from_secs(5), reached.notified())
            .await
            .expect("request reached the provider");
        assert_eq!(provider.semaphore.available_permits(), 0);
        assert!(state.worker.cancel_active_task(&task.task_id));
        let result = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        server.abort();
        assert_eq!(result.unwrap_err(), "task_llm_cancelled");
        assert_eq!(provider.semaphore.available_permits(), 1);
        let events = crate::task_event_transport::replay_events_after(&state, &task.task_id, 0)
            .unwrap()
            .events;
        assert!(events.iter().any(|event| event
            .pointer("/payload/reason_code")
            .and_then(serde_json::Value::as_str)
            == Some("task_llm_cancelled")));
    }
}
