use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::run_with_tool_budget_timeout;

#[tokio::test]
async fn tool_budget_timeout_returns_only_machine_fields() {
    let error = run_with_tool_budget_timeout(Some((0, "short")), None, async {
        std::future::pending::<Result<String, String>>().await
    })
    .await
    .expect_err("pending tool must time out");
    let value: Value = serde_json::from_str(&error).expect("machine timeout object");

    assert_eq!(value["status_code"], "agent_tool_timeout");
    assert_eq!(value["error_code"], "agent_tool_timeout");
    assert_eq!(value["timeout_class"], "short");
    assert_eq!(value["timeout_seconds"], 0);
    assert_eq!(value["resumable"], false);
    assert!(value.get("text").is_none());
    assert!(value.get("error_text").is_none());
}

#[tokio::test]
async fn missing_tool_budget_timeout_does_not_wrap_execution() {
    let output =
        run_with_tool_budget_timeout(None, None, async { Ok("machine-output".to_string()) })
            .await
            .expect("unbounded helper result");

    assert_eq!(output, "machine-output");
}

#[tokio::test]
async fn task_cancellation_preempts_a_pending_tool_call() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = run_with_tool_budget_timeout(None, Some(cancellation), async {
        std::future::pending::<Result<String, String>>().await
    })
    .await
    .expect_err("cancelled tool must stop");

    assert_eq!(error, super::TASK_CANCELED_ERR);
}
