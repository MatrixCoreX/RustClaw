use tempfile::tempdir;

use super::*;

#[test]
fn operations_are_durable_cancelable_and_recover_interrupted_state() {
    let root = tempdir().expect("tempdir");
    let store = SkillOperationStore::new(root.path());
    let queued = store
        .create("demo_skill", OperationAction::Install)
        .expect("create");
    let running = store
        .transition(
            &queued.operation_id,
            OperationStatus::Running,
            OperationStage::Build,
            None,
            None,
        )
        .expect("running");
    assert_eq!(store.get(&running.operation_id).expect("reload"), running);
    assert!(
        store
            .request_cancel(&running.operation_id)
            .expect("cancel")
            .cancel_requested
    );
    let recovered = store.recover_interrupted().expect("recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].status, OperationStatus::Failure);
    assert_eq!(
        recovered[0].failure.as_ref().expect("failure").error_code,
        "operation_interrupted"
    );
}

#[test]
fn terminal_operation_cannot_be_rewritten() {
    let root = tempdir().expect("tempdir");
    let store = SkillOperationStore::new(root.path());
    let queued = store
        .create("demo_skill", OperationAction::Remove)
        .expect("create");
    store
        .transition(
            &queued.operation_id,
            OperationStatus::Success,
            OperationStage::Success,
            None,
            Some(serde_json::json!({"removed": true})),
        )
        .expect("terminal");
    let error = store
        .transition(
            &queued.operation_id,
            OperationStatus::Failure,
            OperationStage::Failure,
            None,
            None,
        )
        .expect_err("terminal immutable");
    assert_eq!(error.code, "operation_already_terminal");
}

#[test]
fn operation_failure_diagnostics_must_be_bounded_and_redacted() {
    let root = tempdir().expect("tempdir");
    let store = SkillOperationStore::new(root.path());
    let queued = store
        .create("demo_skill", OperationAction::Install)
        .expect("create");
    let unsafe_failure = OperationFailure {
        error_code: "dependency_failed".to_string(),
        message_key: "skill_store.dependency_failed".to_string(),
        phase: Some("dependencies".to_string()),
        retryable: true,
        diagnostic: Some("API_TOKEN=do-not-persist".to_string()),
    };
    let error = store
        .transition(
            &queued.operation_id,
            OperationStatus::Failure,
            OperationStage::Failure,
            Some(unsafe_failure),
            None,
        )
        .expect_err("unredacted diagnostic must be rejected");
    assert_eq!(error.code, "operation_diagnostic_unsafe");
    assert_eq!(
        store
            .get(&queued.operation_id)
            .expect("operation intact")
            .status,
        OperationStatus::Queued
    );

    let safe = crate::secret_scan::redact_diagnostics("API_TOKEN=do-not-persist");
    store
        .transition(
            &queued.operation_id,
            OperationStatus::Failure,
            OperationStage::Failure,
            Some(OperationFailure {
                error_code: "dependency_failed".to_string(),
                message_key: "skill_store.dependency_failed".to_string(),
                phase: Some("dependencies".to_string()),
                retryable: true,
                diagnostic: Some(safe.clone()),
            }),
            None,
        )
        .expect("redacted diagnostic accepted");
    let persisted =
        std::fs::read_to_string(store.root().join(format!("{}.json", queued.operation_id)))
            .expect("read durable operation");
    assert!(persisted.contains(&safe));
    assert!(!persisted.contains("do-not-persist"));
}
