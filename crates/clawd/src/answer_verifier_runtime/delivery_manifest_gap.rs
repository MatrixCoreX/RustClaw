use super::*;

/// A grounded path proves a file exists, not that the user can download it.
/// Reuse the actual publication contract before accepting a delivery claim.
pub(super) fn missing_delivery_manifest_gap(
    workspace: &std::path::Path,
    task_id: &str,
    contract: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate: &str,
) -> Option<AnswerVerifierOut> {
    if !route_requires_single_file_delivery(contract)
        || confirmed_missing_file_delivery_can_skip_answer_verifier(contract, journal, candidate)
    {
        return None;
    }
    let result = serde_json::json!({"task_journal":{"trace":{"capability_results":journal.capability_results}}});
    let published = crate::task_artifacts::materialize_task_result_artifacts(
        workspace,
        task_id,
        &result.to_string(),
    )
    .ok()
    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    if published
        .as_ref()
        .and_then(|value| value.get("artifacts"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| !items.is_empty())
    {
        return None;
    }
    Some(AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["task_delivery_artifacts".into()],
        answer_incomplete_reason: "required_delivery_manifest_missing".into(),
        should_retry: true,
        retry_instruction: serde_json::json!({"retry_policy":"publish_current_task_deliverable", "required_evidence":"task_owned_artifact_manifest", "path_only_is_delivery":false, "preserve_completed_effects":true}).to_string(),
        confidence: 1.0,
    })
}

#[cfg(test)]
#[path = "delivery_manifest_gap_tests.rs"]
mod tests;
