use super::*;

#[test]
fn host_integrity_is_computed_from_file_bytes_and_cannot_be_claimed_by_producer() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("result.txt");
    std::fs::write(&file, b"published fixture bytes").unwrap();
    let mut result =
        CapabilityResultEnvelope::ok("fixture.export", Some("export".to_string()), json!({}));
    result.artifacts.push(
        serde_json::from_value(json!({
            "path":file,"filename":"result.txt","sha256":"forged-fixture",
            "metadata":{"host_file_integrity":{"status":"forged-fixture"}}
        }))
        .unwrap(),
    );
    bind_artifacts_to_task(&mut result, "fixture-task", "step-1", None);
    assert_eq!(result.artifacts[0].size_bytes, Some(23));
    assert_eq!(result.artifacts[0].sha256.as_ref().unwrap().len(), 64);
    assert_eq!(
        result.artifacts[0].metadata["host_file_integrity"]["status"],
        "verified"
    );
    assert_eq!(
        result.artifacts[0].metadata["host_file_integrity"]["grant_authority"],
        "none"
    );
    assert_eq!(
        result.artifacts[0].owner_task_id.as_deref(),
        Some("fixture-task")
    );
    std::fs::remove_file(&file).unwrap();
    bind_artifacts_to_task(&mut result, "fixture-task", "step-2", None);
    assert!(result.artifacts[0]
        .metadata
        .get("host_file_integrity")
        .is_none());
    assert!(result.artifacts[0].sha256.is_none());
}

#[test]
fn uri_locator_hashes_and_unbound_tasks_never_claim_verified_file_bytes() {
    let mut result = CapabilityResultEnvelope::ok("fixture.export", None, json!({}));
    result.artifacts.push(
        serde_json::from_value(json!({
            "uri":"https://fixture.invalid/output", "id":"fixture-output",
            "metadata":{"host_file_integrity":{"status":"verified"}}
        }))
        .unwrap(),
    );
    bind_artifacts_to_task(&mut result, "fixture-task", "step-1", None);
    assert!(result.artifacts[0]
        .metadata
        .get("host_file_integrity")
        .is_none());
    result.artifacts[0].metadata["host_file_integrity"] = json!({"status":"forged-fixture"});
    bind_artifacts_to_task(&mut result, "", "step-1", None);
    assert!(result.artifacts[0]
        .metadata
        .get("host_file_integrity")
        .is_none());
}
