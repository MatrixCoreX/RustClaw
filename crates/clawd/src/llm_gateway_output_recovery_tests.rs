use super::*;
use axum::{routing::post, Json, Router};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

struct Fixture {
    state: AppState,
    task: ClaimedTask,
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new(length_responses: usize, total_tokens: u64) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let app = Router::new().route("/chat/completions", post(move |Json(body): Json<serde_json::Value>| {
            let captured = captured.clone();
            async move {
                let (index, output_cap) = {
                    let mut rows = captured.lock().unwrap();
                    let index = rows.len();
                    let cap = body["max_tokens"].as_u64().unwrap();
                    rows.push(body);
                    (index, cap)
                };
                let length = index < length_responses;
                let output = if length { output_cap } else { 64 };
                Json(json!({
                    "model":"MiniMax-M3",
                    "choices":[{"message":{"role":"assistant","content":"","tool_calls":[{
                        "id":format!("fixture-call-{index}"),"type":"function",
                        "function":{"name":if length { "fixture_mutation" } else { "fixture_complete" },"arguments":"{}"}
                    }]},"finish_reason":if length { "length" } else { "tool_calls" }}],
                    "usage":{"prompt_tokens":16,"completion_tokens":output,"total_tokens":16+output}
                }))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let template = state.core.llm_providers[0].clone();
        let mut config = template.config.clone();
        config.name = "vendor-minimax".into();
        config.provider_type = "openai_compat".into();
        config.model = "MiniMax-M3".into();
        config.supports_tools = true;
        config.base_url = format!("http://{address}");
        config.timeout_seconds = 10;
        config.params.stream = Some(false);
        state.core.llm_providers = vec![Arc::new(crate::LlmProviderRuntime {
            config,
            pricing: None,
            latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
            client: crate::providers::build_llm_http_client(10).unwrap(),
            semaphore: Arc::new(Semaphore::new(1)),
            breaker: Arc::new(crate::providers::CircuitBreaker::new()),
        })];
        let id = uuid::Uuid::new_v4().to_string();
        state.seed_ask_task_row(&id, 7, 11, &json!({
            "text":"fixture request","child_task_contract":{"budget":{"max_tokens":total_tokens}}
        }).to_string());
        let task = crate::repo::claim_next_task(&state).unwrap().unwrap();
        state.worker.register_active_task(&task.task_id);
        Self {
            state,
            task,
            requests,
            server,
        }
    }

    async fn call(&self) -> Result<Option<ModelTurnResponse>, String> {
        let mut request = ModelTurnRequest::text("fixture request");
        request.stream = false;
        request.tools = ["fixture_mutation", "fixture_complete"]
            .into_iter()
            .map(|name| claw_core::model_turn::ModelToolDefinition {
                name: name.into(),
                description: "fixture".into(),
                input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
                strict: true,
            })
            .collect();
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run_native_model_turn_with_fallback(
                &self.state,
                &self.task,
                "fixture request",
                "test:native_output_limit_recovery",
                &request,
            ),
        )
        .await
        .expect("bounded gateway fixture")
    }

    fn limits(&self) -> Vec<u64> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|row| row["max_tokens"].as_u64().unwrap())
            .collect()
    }
}

#[tokio::test]
async fn completed_turn_keeps_existing_budget_and_is_not_retried() {
    let fixture = Fixture::new(0, 1000000).await;
    let turn = fixture.call().await.unwrap().unwrap();
    assert_eq!(turn.finish_reason, ModelFinishReason::ToolCalls);
    assert_eq!(fixture.limits(), vec![8192]);
}

#[tokio::test]
async fn length_discards_complete_partial_calls_then_recovers_without_disabling_thinking() {
    let fixture = Fixture::new(1, 1000000).await;
    let turn = fixture.call().await.unwrap().unwrap();
    assert_eq!(fixture.limits(), vec![8192, 16384]);
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "fixture_complete");
    assert!(fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|row| row.get("thinking").is_none()));
    let db = fixture.state.core.db.get().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM llm_cost_ledger WHERE task_id=?1",
            [&fixture.task.task_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM task_model_token_reservations WHERE task_id=?1 AND status='in_flight'", [&fixture.task.task_id], |row|row.get::<_,i64>(0)).unwrap(), 0);
    let events =
        crate::task_event_transport::replay_events_after(&fixture.state, &fixture.task.task_id, 0)
            .unwrap()
            .events;
    assert!(events
        .iter()
        .any(|event| event.pointer("/payload/next_output_tokens") == Some(&json!(16384))));
}

#[tokio::test]
async fn repeated_length_stops_at_configured_attempt_and_generation_ceiling() {
    let fixture = Fixture::new(usize::MAX, 1000000).await;
    assert_eq!(fixture.call().await.unwrap_err(), EXHAUSTED);
    assert_eq!(fixture.limits(), vec![8192, 16384, 32768]);
}

#[tokio::test]
async fn exhausted_task_budget_blocks_growth_before_another_provider_request() {
    let fixture = Fixture::new(1, 20000).await;
    let error = fixture.call().await.unwrap_err();
    let detail: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(detail["error_code"], "administrator_token_budget_exhausted");
    assert_eq!(detail["provider_called"], false);
    assert_eq!(fixture.limits(), vec![8192]);
}

#[tokio::test]
async fn explicit_model_output_policy_cannot_be_raised_by_recovery() {
    let mut fixture = Fixture::new(1, 1000000).await;
    let mut payload: serde_json::Value = serde_json::from_str(&fixture.task.payload_json).unwrap();
    payload["resolved_model_policy"] = json!({"max_output_tokens":8192});
    fixture.task.payload_json = payload.to_string();
    let error = fixture.call().await.unwrap_err();
    let detail: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(detail["error_code"], "model_output_policy_limit");
    assert_eq!(detail["provider_called"], false);
    assert_eq!(fixture.limits(), vec![8192]);
}

#[test]
fn configured_growth_is_monotonic_bounded_and_can_be_disabled() {
    let mut policy = ModelOutputBudgetPolicy {
        default_tokens: 6000,
        recovery_max_tokens: 9000,
        recovery_attempts: 1,
        ..Default::default()
    };
    let mut budget = GenerationBudget::new(policy, "plan");
    assert!(budget.hints().minimum_output_tokens.is_none());
    assert!(budget.grow());
    assert_eq!(budget.tokens, 9000);
    assert_eq!(budget.hints().minimum_output_tokens, Some(9000));
    assert!(!budget.grow());
    policy.recovery_attempts = 0;
    assert!(!GenerationBudget::new(policy, "plan").grow());
    policy.recovery_attempts = 2;
    policy.recovery_max_tokens = 6000;
    assert!(!GenerationBudget::new(policy, "plan").grow());
}
