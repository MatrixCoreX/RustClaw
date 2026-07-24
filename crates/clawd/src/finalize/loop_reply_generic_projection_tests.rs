use claw_core::capability_result::{ArtifactRef, CapabilityResultEnvelope, EvidenceRef};
use serde_json::json;

use super::{
    candidate_contradicts_projection, project_capability_results, GenericProjection,
    GenericProjectionIssueCode,
};

fn result(data: serde_json::Value) -> CapabilityResultEnvelope {
    let mut result = CapabilityResultEnvelope::ok("fs_basic", Some("inspect".to_string()), data);
    result.evidence.push(EvidenceRef {
        id: "step_1".to_string(),
        source: "fs_basic".to_string(),
        locator: None,
        digest: None,
        metadata: json!({}),
    });
    result
}

fn scalar_contract(selector: &str) -> crate::IntentOutputContract {
    let mut contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        ..Default::default()
    };
    contract.selection.structured_field_selector = Some(selector.to_string());
    contract
}

#[test]
fn projects_one_exact_scalar_from_capability_data() {
    let projection = project_capability_results(
        &scalar_contract("count"),
        &[result(json!({"output": {"count": 7}}))],
    );
    assert_eq!(
        projection,
        GenericProjection::Projected {
            text: "7".to_string(),
            evidence_count: 1,
        }
    );
}

#[test]
fn projects_named_fields_as_stable_json_object() {
    let mut contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        ..Default::default()
    };
    contract.selection.structured_field_selector = Some("exists,path".to_string());
    let projection = project_capability_results(
        &contract,
        &[result(json!({
            "output": {
                "exists": true,
                "path": "/workspace/report.txt"
            }
        }))],
    );
    assert_eq!(
        projection,
        GenericProjection::Projected {
            text: r#"{"exists":true,"path":"/workspace/report.txt"}"#.to_string(),
            evidence_count: 1,
        }
    );
}

#[test]
fn projects_selected_array_with_contract_limit() {
    let mut contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        ..Default::default()
    };
    contract.selection.structured_field_selector = Some("paths".to_string());
    contract.selection.list_selector.target_kind =
        crate::pipeline_types::OutputScalarCountTargetKind::File;
    contract.selection.list_selector.target_kind_specified = true;
    contract.selection.list_selector.limit = Some(2);
    let projection = project_capability_results(
        &contract,
        &[result(json!({
            "output": {"paths": ["/workspace/a", "/workspace/b", "/workspace/c"]}
        }))],
    );
    assert_eq!(
        projection,
        GenericProjection::Projected {
            text: "/workspace/a\n/workspace/b".to_string(),
            evidence_count: 1,
        }
    );
}

#[test]
fn rejects_ambiguous_values_across_observations() {
    let projection = project_capability_results(
        &scalar_contract("count"),
        &[
            result(json!({"output": {"count": 2}})),
            result(json!({"output": {"count": 3}})),
        ],
    );
    assert!(matches!(
        projection,
        GenericProjection::RepairIssue(issue)
            if issue.code == GenericProjectionIssueCode::AmbiguousValue
                && issue.selectors == ["count"]
    ));
}

#[test]
fn exact_list_requires_an_explicit_machine_selector() {
    let mut contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        ..Default::default()
    };
    contract.selection.list_selector.target_kind =
        crate::pipeline_types::OutputScalarCountTargetKind::File;
    contract.selection.list_selector.target_kind_specified = true;
    let projection = project_capability_results(&contract, &[result(json!({"output": []}))]);
    assert!(matches!(
        projection,
        GenericProjection::RepairIssue(issue)
            if issue.code == GenericProjectionIssueCode::MissingSelector
    ));
}

#[test]
fn projects_one_artifact_path_as_file_token() {
    let contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::FileToken,
        delivery_required: true,
        delivery_intent: crate::OutputDeliveryIntent::FileSingle,
        ..Default::default()
    };
    let mut envelope = result(json!({"output": {"status": "created"}}));
    envelope.artifacts.push(ArtifactRef {
        id: Some("report".to_string()),
        path: Some("/workspace/report.docx".to_string()),
        uri: None,
        media_type: None,
        sha256: None,
        metadata: json!({}),
    });
    assert_eq!(
        project_capability_results(&contract, &[envelope]),
        GenericProjection::Projected {
            text: "FILE:/workspace/report.docx".to_string(),
            evidence_count: 1,
        }
    );
}

#[test]
fn detects_contradictory_machine_delivery_without_language_parsing() {
    assert!(!candidate_contradicts_projection(
        r#"{"count":2}"#,
        r#"{ "count": 2 }"#
    ));
    assert!(candidate_contradicts_projection("3", "2"));
}
