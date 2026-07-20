use std::{collections::BTreeMap, path::PathBuf};

use super::*;
use crate::task_contract::fallback_required_evidence_fields_for_output_contract;
use crate::{OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape};
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn load_workspace_matrix() -> ContractMatrix {
    ContractMatrix::load_from_workspace(&workspace_root()).expect("load contract matrix")
}

#[test]
fn generic_path_content_allows_runtime_equivalent_config_guard() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "config_edit",
        &serde_json::json!({
            "action": "guard_config",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("generic path content profile should classify config guard");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.guard_rustclaw_config");
    assert_eq!(policy.contract_match, "generic_path_content");
}

fn load_registry_from_text(raw: &str) -> SkillsRegistry {
    let path = std::env::temp_dir().join(format!(
        "contract_matrix_test_registry_{}_{}.toml",
        std::process::id(),
        fnv1a_hex(raw)
    ));
    std::fs::write(&path, raw).expect("write registry fixture");
    let registry = SkillsRegistry::load_from_path(&path).expect("load registry fixture");
    let _ = std::fs::remove_file(path);
    registry
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GeneratedContractMatch {
    Semantic(OutputSemanticKind),
    Generic(String),
}

#[derive(Debug, Clone)]
struct GeneratedMatrixCase {
    id: String,
    matched: GeneratedContractMatch,
    action: Option<ActionRef>,
    expected_decision: Option<ActionPolicyDecision>,
    expected_required_evidence: Vec<String>,
    expected_final_answer_shape: String,
}

fn generated_allowed_action(matched: &MatchedContract<'_>) -> Option<ActionRef> {
    for raw in matched.allowed_actions() {
        let action = ActionRef::parse(raw)?;
        if matched.action_policy(&action) == ActionPolicyDecision::Allowed {
            return Some(action);
        }
    }
    if matches!(
        matched,
        MatchedContract::Semantic(MatrixContract {
            none_passthrough: true,
            ..
        })
    ) {
        return ActionRef::parse("respond");
    }
    None
}

fn generated_negative_action(
    matched: &MatchedContract<'_>,
) -> Option<(ActionRef, ActionPolicyDecision)> {
    for raw in matched.forbidden_actions() {
        let action = ActionRef::parse(raw)?;
        let decision = matched.action_policy(&action);
        if decision != ActionPolicyDecision::Allowed {
            return Some((action, decision));
        }
    }

    let probes = [
        "run_cmd",
        "fs_basic.list_dir",
        "fs_basic.read_text_range",
        "fs_basic.write_text",
        "archive_basic.pack",
        "config_basic.validate",
        "docker_basic",
        "package_manager.detect",
        "db_basic",
        "health_check",
        "respond",
    ];
    for probe in probes {
        let action = ActionRef::parse(probe).expect("probe action parses");
        let decision = matched.action_policy(&action);
        if decision != ActionPolicyDecision::Allowed {
            return Some((action, decision));
        }
    }
    None
}

fn push_generated_case(
    cases: &mut Vec<GeneratedMatrixCase>,
    id: String,
    matched: GeneratedContractMatch,
    contract: &MatchedContract<'_>,
    action: Option<ActionRef>,
    expected_decision: Option<ActionPolicyDecision>,
) {
    cases.push(GeneratedMatrixCase {
        id,
        matched,
        action,
        expected_decision,
        expected_required_evidence: contract.required_evidence(),
        expected_final_answer_shape: contract.final_answer_shape().to_string(),
    });
}

fn generated_contract_cases(
    matrix: &ContractMatrix,
    minimum_count: usize,
) -> Vec<GeneratedMatrixCase> {
    let mut cases = Vec::new();

    for kind in OutputSemanticKind::ALL {
        let contract = matrix
            .semantic_contract(*kind)
            .expect("semantic contract exists");
        let matched = MatchedContract::Semantic(contract);
        let case_match = GeneratedContractMatch::Semantic(*kind);
        let prefix = kind.as_str();

        push_generated_case(
            &mut cases,
            format!("{prefix}::evidence_shape"),
            case_match.clone(),
            &matched,
            None,
            None,
        );

        if let Some(action) = generated_allowed_action(&matched) {
            let decision = matched.action_policy(&action);
            push_generated_case(
                &mut cases,
                format!("{prefix}::allowed::{}", action.as_key()),
                case_match.clone(),
                &matched,
                Some(action),
                Some(decision),
            );
        }

        if let Some((action, decision)) = generated_negative_action(&matched) {
            push_generated_case(
                &mut cases,
                format!("{prefix}::negative::{}", action.as_key()),
                case_match,
                &matched,
                Some(action),
                Some(decision),
            );
        }
    }

    for profile in &matrix.generic_profiles {
        let matched = MatchedContract::Generic(profile);
        let case_match = GeneratedContractMatch::Generic(profile.name.clone());
        let prefix = format!("generic::{}", profile.name);

        push_generated_case(
            &mut cases,
            format!("{prefix}::evidence_shape"),
            case_match.clone(),
            &matched,
            None,
            None,
        );

        if let Some(action) = generated_allowed_action(&matched) {
            let decision = matched.action_policy(&action);
            push_generated_case(
                &mut cases,
                format!("{prefix}::allowed::{}", action.as_key()),
                case_match.clone(),
                &matched,
                Some(action),
                Some(decision),
            );
        }

        if let Some((action, decision)) = generated_negative_action(&matched) {
            push_generated_case(
                &mut cases,
                format!("{prefix}::negative::{}", action.as_key()),
                case_match,
                &matched,
                Some(action),
                Some(decision),
            );
        }
    }

    assert!(
        cases.len() >= minimum_count,
        "matrix generated only {} cases, expected at least {minimum_count}",
        cases.len()
    );
    cases
}

fn matched_for_generated_case<'a>(
    matrix: &'a ContractMatrix,
    case: &GeneratedMatrixCase,
) -> MatchedContract<'a> {
    match &case.matched {
        GeneratedContractMatch::Semantic(kind) => MatchedContract::Semantic(
            matrix
                .semantic_contract(*kind)
                .expect("semantic contract exists"),
        ),
        GeneratedContractMatch::Generic(name) => MatchedContract::Generic(
            matrix
                .generic_profiles
                .iter()
                .find(|profile| profile.name == *name)
                .expect("generic profile exists"),
        ),
    }
}

#[test]
fn workspace_contract_matrix_loads_and_has_shape() {
    let matrix = load_workspace_matrix();

    assert!(matrix.validate_shape().is_empty());
    assert_eq!(matrix.schema_version, 1);
    assert!(!matrix.matrix_version_hash().is_empty());
    assert!(matrix
        .failure_attribution
        .contains(&"model_error".to_string()));
    assert_eq!(matrix.policy.unknown_semantic, "reject");
    assert_eq!(
        matrix.trace_policy.evidence_storage,
        "redacted_excerpt_hash"
    );
    assert_eq!(
        matrix.trace_policy.provider_evidence_view,
        "provider_safe_redacted"
    );
}

#[test]
fn delete_contracts_cannot_be_satisfied_by_read_or_list_actions() {
    let matrix = load_workspace_matrix();
    for (name, contract) in &matrix.contracts {
        assert_delete_policy_for_actions(
            &format!("contract `{name}`"),
            &contract.operation,
            &contract.allowed_actions,
        );
    }
    for profile in &matrix.generic_profiles {
        for raw in &profile.allowed_actions {
            let Some(action) = ActionRef::parse(raw) else {
                continue;
            };
            assert!(
                !action_is_delete_mutation(&action),
                "generic profile `{}` allows delete action `{}`",
                profile.name,
                action.as_key()
            );
        }
    }
}

fn assert_delete_policy_for_actions(context: &str, operation: &str, actions: &[String]) {
    let operation = normalize_action_token(operation);
    for raw in actions {
        let Some(action) = ActionRef::parse(raw) else {
            continue;
        };
        if operation == "delete" {
            assert!(
                !action_is_read_or_list_observation(&action),
                "{context} allows read/list observation action `{}` for delete operation",
                action.as_key()
            );
        } else if operation != "mutate" {
            assert!(
                !action_is_delete_mutation(&action),
                "{context} allows delete action `{}` without delete operation",
                action.as_key()
            );
        }
    }
}

fn action_is_delete_mutation(action: &ActionRef) -> bool {
    matches!(
        (action.skill.as_str(), action.action.as_deref()),
        ("fs_basic", Some("remove_path")) | ("remove_file", _)
    )
}

fn action_is_read_or_list_observation(action: &ActionRef) -> bool {
    matches!(
        (action.skill.as_str(), action.action.as_deref()),
        ("read_file" | "list_dir" | "doc_parse", _)
            | (
                "fs_basic",
                Some("list_dir" | "read_text_range" | "find_entries" | "grep_text")
            )
            | ("archive_basic", Some("list" | "read"))
            | (
                "config_basic",
                Some("read_field" | "read_fields" | "list_keys")
            )
            | ("db_basic", Some("list_tables" | "query"))
    )
}

#[test]
fn failure_attribution_enum_matches_workspace_matrix() {
    let matrix = load_workspace_matrix();
    let configured = matrix
        .failure_attribution
        .iter()
        .filter_map(|value| FailureAttribution::parse(value))
        .collect::<BTreeSet<_>>();
    let expected = FailureAttribution::ALL.into_iter().collect::<BTreeSet<_>>();

    assert_eq!(configured, expected);
}

#[test]
fn failure_attribution_rejects_unknown_tokens() {
    let mut matrix = ContractMatrix {
        schema_version: 1,
        matrix_version: "test".to_string(),
        failure_attribution: FailureAttribution::ALL
            .iter()
            .map(|kind| kind.as_str().to_string())
            .chain(std::iter::once("mystery_bucket".to_string()))
            .collect(),
        ..Default::default()
    };
    matrix.trace_policy = MatrixTracePolicy {
        evidence_storage: "redacted_excerpt_hash".to_string(),
        provider_evidence_view: "provider_safe_redacted".to_string(),
        raw_excerpt_policy: "no_full_raw_excerpt".to_string(),
        max_items: 24,
        max_excerpt_chars: 240,
    };

    let errors = matrix.validate_shape();

    assert!(errors
        .iter()
        .any(|error| error == "invalid failure attribution `mystery_bucket`"));
}

#[test]
fn bundled_contract_matrix_result_exposes_load_errors() {
    let matrix = bundled_contract_matrix_result().expect("bundled matrix should load");

    assert_eq!(matrix.schema_version, 1);

    let err = parse_contract_matrix_source(
        r#"schema_version = 1
matrix_version = "broken"
"#,
    )
    .expect_err("invalid matrix should report a concrete error");
    assert!(err.contains("contract matrix shape invalid"));
    assert!(err.contains("missing failure attribution"));
}

#[test]
fn existence_contract_can_express_negative_evidence() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ExistenceWithPath)
        .expect("existence contract");
    let expression = contract.evidence_expression();

    assert_eq!(expression.all_of, vec!["kind", "path"]);
    assert_eq!(expression.one_of, vec!["exists_false", "exists_true"]);
    assert_eq!(expression.negative_evidence, vec!["exists_false"]);
}

#[test]
fn trace_snapshot_includes_evidence_expression_trace_policy_and_sources() {
    let mut output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        ..IntentOutputContract::default()
    };
    output_contract.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    output_contract
        .selection
        .list_selector
        .target_kind_specified = true;
    let snapshot = trace_snapshot_for_output_contract(&output_contract).expect("trace snapshot");

    assert_eq!(
        snapshot
            .get("trace_policy")
            .and_then(|value| value.get("provider_evidence_view"))
            .and_then(Value::as_str),
        Some("provider_safe_redacted")
    );
    assert_eq!(
        snapshot.get("policy_mode").and_then(Value::as_str),
        Some("enforce")
    );
    assert_eq!(
        snapshot.get("contract_marker").and_then(Value::as_str),
        Some("none")
    );
    assert!(snapshot.get("semantic_kind").is_none());
    assert_eq!(
        snapshot.get("evidence_scope").and_then(Value::as_str),
        Some("current_task")
    );
    assert_eq!(
        snapshot.get("freshness").and_then(Value::as_str),
        Some("current_task")
    );
    assert_eq!(
        snapshot.get("artifact_kind").and_then(Value::as_str),
        Some("text")
    );
    assert_eq!(
        snapshot.get("channel_visibility").and_then(Value::as_str),
        Some("user_visible")
    );
    assert_eq!(
        snapshot.get("evidence_profile").and_then(Value::as_str),
        Some("generic")
    );
    assert_eq!(
        snapshot
            .get("evidence_expression")
            .and_then(|value| value.get("all_of"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("candidates")
    );
    assert!(snapshot
        .get("observation_sources")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("fs_basic.list_dir"))));
    assert!(snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| {
            item.get("source").and_then(Value::as_str) == Some("fs_basic.list_dir")
                && item.get("extractor_kind").and_then(Value::as_str) == Some("structured_json")
        })));
}

#[test]
fn raw_command_observation_source_defaults_to_text_legacy_extractor() {
    let snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        ..IntentOutputContract::default()
    })
    .expect("trace snapshot");

    assert!(snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| {
            item.get("source").and_then(Value::as_str) == Some("run_cmd")
                && item.get("extractor_kind").and_then(Value::as_str) == Some("text_legacy")
        })));
}

#[test]
fn archive_count_uses_structured_action_observation() {
    let scalar_snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::None,
        selection: crate::OutputSelectionContract {
            structured_field_selector: Some("count".to_string()),
            ..Default::default()
        },
        ..IntentOutputContract::default()
    })
    .expect("scalar trace snapshot");
    let scalar_extractors = scalar_snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .expect("scalar observation extractors");
    assert!(!scalar_extractors.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("archive_basic")
            && item.get("extractor_kind").and_then(Value::as_str) == Some("text_legacy")
    }));

    let scalar_contract = IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::None,
        selection: crate::OutputSelectionContract {
            structured_field_selector: Some("count".to_string()),
            ..Default::default()
        },
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let archive_trace = action_trace_for_output_contract(&scalar_contract, "archive_basic.list")
        .expect("archive output-contract action trace");
    assert_eq!(
        archive_trace
            .get("observation_extractor")
            .and_then(|item| item.get("source"))
            .and_then(Value::as_str),
        Some("archive_basic.list")
    );
    assert_eq!(
        archive_trace
            .get("observation_extractor")
            .and_then(|item| item.get("extractor_kind"))
            .and_then(Value::as_str),
        Some("structured_json")
    );
    assert_eq!(
        archive_trace.get("contract_match").and_then(Value::as_str),
        Some("generic_exact_count")
    );
}

#[test]
fn planner_semantic_kinds_directly_match_their_contracts() {
    let matrix = load_workspace_matrix();

    for kind in OutputSemanticKind::ALL
        .iter()
        .filter(|kind| **kind != OutputSemanticKind::None)
    {
        let output_contract = IntentOutputContract {
            semantic_kind: *kind,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        };
        let matched = matrix
            .match_output_contract(&output_contract)
            .unwrap_or_else(|| panic!("semantic match for {}", kind.as_str()));
        assert_eq!(
            matched.match_name(),
            kind.as_str(),
            "{} must directly own planner contract policy",
            kind.as_str()
        );
    }
}

#[test]
fn generic_delivery_snapshot_defaults_to_file_artifact() {
    let snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        delivery_required: true,
        ..IntentOutputContract::default()
    })
    .expect("trace snapshot");

    assert_eq!(
        snapshot.get("contract_match").and_then(Value::as_str),
        Some("generic_delivery")
    );
    assert_eq!(
        snapshot.get("artifact_kind").and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn action_trace_records_contract_decision_and_shape() {
    let mut output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        ..IntentOutputContract::default()
    };
    output_contract.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    output_contract
        .selection
        .list_selector
        .target_kind_specified = true;
    let trace = action_trace_for_output_contract(&output_contract, "fs_basic.list_dir")
        .expect("action trace should resolve");

    assert_eq!(
        trace.get("contract_match").and_then(Value::as_str),
        Some("generic_path_content")
    );
    assert_eq!(
        trace.get("decision").and_then(Value::as_str),
        Some("allowed")
    );
    assert_eq!(
        trace.get("final_answer_shape").and_then(Value::as_str),
        Some("exact_list")
    );
    assert_eq!(
        trace.get("evidence_profile").and_then(Value::as_str),
        Some("generic")
    );
    assert_eq!(
        trace
            .get("required_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["candidates"])
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/extractor_kind")
            .and_then(Value::as_str),
        Some("structured_json")
    );
}

#[test]
fn action_trace_marks_run_cmd_extractor_as_text_legacy() {
    let trace = action_trace_for_output_contract(
        &IntentOutputContract {
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            ..IntentOutputContract::default()
        },
        "run_cmd",
    )
    .expect("action trace should resolve");

    assert_eq!(
        trace.get("decision").and_then(Value::as_str),
        Some("allowed")
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/source")
            .and_then(Value::as_str),
        Some("run_cmd")
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/extractor_kind")
            .and_then(Value::as_str),
        Some("text_legacy")
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/registry/extractor_ref")
            .and_then(Value::as_str),
        Some("run_cmd.text_legacy_v1")
    );
}

#[path = "contract_matrix_tests/runtime_policy_and_generic_profiles.rs"]
mod runtime_policy_and_generic_profiles;

#[path = "contract_matrix_tests/artifact_and_external_policy.rs"]
mod artifact_and_external_policy;

#[path = "contract_matrix_tests/action_policy_registry.rs"]
mod action_policy_registry;
