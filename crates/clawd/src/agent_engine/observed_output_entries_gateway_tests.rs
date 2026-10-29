use super::{step, task};
use crate::agent_engine::LoopState;
use claw_core::capability_result::CapabilityResultEnvelope;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn terminal_fallback_sends_complete_canonical_evidence_to_the_provider() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let router = axum::Router::new().fallback(move |axum::Json(body): axum::Json<Value>| {
        let captured = captured.clone();
        async move {
            captured.lock().unwrap().push(body);
            let answer = json!({"answer":"Verified the earlier probe and the final check.",
                "qualified":true,"needs_clarify":false,"is_meta_instruction":false,
                "publishable":true,"confidence":1.0,"reason":"fixture_verified"});
            axum::Json(json!({"choices":[{"message":{"role":"assistant",
                "content":answer.to_string()},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let mut app = crate::AppState::test_default_with_fixture_provider()
        .with_seeded_db_schema()
        .with_prompt_layers_installed();
    let mut provider =
        crate::providers::fixture_replay::build_fixture_replay_runtime("finalization-fixture");
    let runtime = Arc::get_mut(&mut provider).unwrap();
    runtime.config.provider_type = "openai_compat".to_string();
    runtime.config.base_url = base;
    runtime.client = reqwest::Client::builder().no_proxy().build().unwrap();
    app.core.llm_providers = vec![provider];
    app.core.active_provider_type = Some("openai_compat".to_string());
    let mut state = LoopState::new();
    for index in 0..12 {
        state.capability_results.push(CapabilityResultEnvelope::ok(
            "fixture.inspect",
            Some("inspect".to_string()),
            json!({"ordinal":index,"observed_fact":format!("observed_{index}"),
                "api_key":"fixture_credential_must_not_escape"}),
        ));
        state.executed_step_results.push(step(
            &format!("step_{index}"),
            "read_file",
            &format!("unbound_raw_claim_{index}"),
        ));
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::agent_engine::observed_output::try_synthesize_answer_from_observed_output(
            &app,
            &task(),
            "Summarize the observed workflow results in English.",
            &state,
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(output.0, "Verified the earlier probe and the final check.");
    assert!(output.1.contract_ok);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].get("tools").is_none());
    let prompt = requests[0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!prompt.contains("unbound_raw_claim"));
    assert!(!prompt.contains("fixture_credential_must_not_escape"));
    let (_, tail) = prompt.split_once("canonical_capability_evidence=").unwrap();
    let mut decoder = serde_json::Deserializer::from_str(tail);
    let catalog = Value::deserialize(&mut decoder).unwrap();
    assert_eq!(catalog["result_count"], 12);
    assert_eq!(catalog["model_view_complete"], true);
    for index in 0..12 {
        assert_eq!(
            catalog["entries"][index]["model_view"]["result"]["data"]["ordinal"],
            index
        );
    }
}
