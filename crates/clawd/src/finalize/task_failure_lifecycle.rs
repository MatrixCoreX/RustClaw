use serde_json::{json, Value};

pub(super) fn failed_task_lifecycle_payload(err_text: &str) -> Value {
    let failure_attribution = crate::task_journal::failure_attribution_for_error_text(err_text)
        .map(|kind| kind.as_str().to_string())
        .unwrap_or_else(|| "runtime_error".to_string());
    let mut payload = json!({
        "schema_version": 1,
        "state": "failed",
        "source": "ask_failure_finalize",
        "can_poll": true,
        "can_cancel": false,
        "failure_attribution": failure_attribution,
    });
    if let Some(terminal_reason) = terminal_failure_reason_for_error(err_text, &failure_attribution)
    {
        payload["terminal_reason"] = json!(terminal_reason);
    }
    payload
}

fn terminal_failure_reason_for_error(
    err_text: &str,
    failure_attribution: &str,
) -> Option<&'static str> {
    if let Some(structured) = crate::skills::parse_structured_skill_error(err_text.trim()) {
        let error_kind = structured.error_kind.trim().to_ascii_lowercase();
        if matches!(error_kind.as_str(), "timeout" | "idle_timeout") {
            return Some(
                crate::task_lifecycle::TerminalFailureReason::ToolTimeoutWithoutAsyncResume
                    .status_code(),
            );
        }
        if error_kind == "confirmation_timeout" {
            return Some(
                crate::task_lifecycle::TerminalFailureReason::ConfirmationTimeout.status_code(),
            );
        }
    }
    match failure_attribution {
        "provider_error" => Some(
            crate::task_lifecycle::TerminalFailureReason::ProviderWindowExhausted.status_code(),
        ),
        "answer_verifier_gap" | "contract_gap" => {
            Some(crate::task_lifecycle::TerminalFailureReason::VerifierUnrecoverable.status_code())
        }
        _ => None,
    }
}
