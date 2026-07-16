use std::{collections::BTreeMap, path::PathBuf};

use super::*;
use crate::task_contract::fallback_required_evidence_fields_for_output_contract;
use crate::{OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape};
#[path = "contract_matrix_recent_artifacts_tests.rs"]
mod recent_artifacts_tests;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn load_workspace_matrix() -> ContractMatrix {
    ContractMatrix::load_from_workspace(&workspace_root()).expect("load contract matrix")
}

#[test]
fn recent_scalar_equality_allows_structured_field_extractors() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::RecentScalarEqualityCheck)
        .expect("recent scalar equality contract");
    let matched = MatchedContract::Semantic(contract);
    for action in [
        "config_basic.read_field",
        "config_basic.read_fields",
        "system_basic.info",
        "system_basic.runtime_status",
    ] {
        let action_ref = ActionRef::parse(action).expect("action parses");
        assert_eq!(
            matched.action_policy(&action_ref),
            ActionPolicyDecision::Allowed,
            "{action} should be allowed for scalar field and local runtime observations"
        );
    }
}

#[test]
fn command_output_summary_allows_structured_field_extractors() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::CommandOutputSummary)
        .expect("command output summary contract");
    let matched = MatchedContract::Semantic(contract);
    for action in [
        "config_basic.read_field",
        "config_basic.read_fields",
        "fs_basic.stat_paths",
        "fs_basic.read_text_range",
        "archive_basic.list",
        "archive_basic.read",
        "db_basic.list_tables",
    ] {
        let action_ref = ActionRef::parse(action).expect("action parses");
        assert_eq!(
            matched.action_policy(&action_ref),
            ActionPolicyDecision::Allowed,
            "{action} should be allowed for mixed evidence summaries"
        );
    }
    let output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    for (skill, args, expected_action) in [
        (
            "system_basic",
            serde_json::json!({"action":"extract_field","path":"Cargo.toml","field_path":"package.name"}),
            "config_basic.read_field",
        ),
        (
            "system_basic",
            serde_json::json!({"action":"extract_fields","path":"Cargo.toml","field_paths":["package.name"]}),
            "config_basic.read_fields",
        ),
        (
            "system_basic",
            serde_json::json!({"action":"path_batch_facts","paths":["README.md"]}),
            "fs_basic.stat_paths",
        ),
        (
            "archive_basic",
            serde_json::json!({"action":"list","archive":"tmp/test_bundle.zip"}),
            "archive_basic.list",
        ),
        (
            "archive_basic",
            serde_json::json!({"action":"read","archive":"tmp/test_bundle.zip","member":"notes.txt"}),
            "archive_basic.read",
        ),
        (
            "db_basic",
            serde_json::json!({"action":"list_tables","db_path":"data/test_contract.sqlite"}),
            "db_basic.list_tables",
        ),
    ] {
        let policy = action_policy_for_output_contract(Some(&output_contract), skill, &args)
            .expect("runtime-equivalent policy decision");
        assert!(policy.is_allowed(), "{policy:?}");
        assert!(policy.action_matches_preferred(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "command_output_summary");
    }
    let evidence = matched.evidence_expression();
    assert!(evidence.any_of.contains(&"exists".to_string()));
    assert!(evidence.any_of.contains(&"path".to_string()));
}

#[test]
fn content_excerpt_summary_allows_runtime_equivalent_config_guard() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
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
    .expect("content excerpt contract should classify config guard");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.guard_rustclaw_config");
    assert_eq!(policy.contract_match, "content_excerpt_summary");
}

#[test]
fn archive_list_allows_compound_readonly_archive_and_db_observations() {
    for (semantic_kind, skill, args, expected_action, expected_evidence) in [
        (
            OutputSemanticKind::ArchiveList,
            "archive_basic",
            serde_json::json!({"action":"list","archive":"tmp/test_bundle.zip"}),
            "archive_basic.list",
            vec!["candidates"],
        ),
        (
            OutputSemanticKind::ArchiveRead,
            "archive_basic",
            serde_json::json!({"action":"read","archive":"tmp/test_bundle.zip","member":"notes.txt"}),
            "archive_basic.read",
            vec!["content_excerpt"],
        ),
        (
            OutputSemanticKind::SqliteTableListing,
            "db_basic",
            serde_json::json!({"action":"list_tables","db_path":"data/test_contract.sqlite"}),
            "db_basic.list_tables",
            vec!["candidates"],
        ),
    ] {
        let output_contract = IntentOutputContract {
            semantic_kind,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };
        let policy = action_policy_for_output_contract(Some(&output_contract), skill, &args)
            .unwrap_or_else(|| panic!("output-contract policy decision for {expected_action}"));
        assert!(policy.is_allowed(), "{policy:?}");
        assert!(policy.action_matches_preferred(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, semantic_kind.as_str());
        assert_eq!(policy.required_evidence, expected_evidence);
    }
}

#[test]
fn service_status_allows_http_basic_observation() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ServiceStatus)
        .expect("service status contract");
    let matched = MatchedContract::Semantic(contract);
    let action_ref = ActionRef::parse("http_basic.get").expect("action parses");

    assert_eq!(
        matched.action_policy(&action_ref),
        ActionPolicyDecision::Allowed
    );
}

#[test]
fn rss_news_fetch_allows_rss_fetch_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RssNewsFetch,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "rss_fetch",
        &serde_json::json!({"action":"latest","category":"general","limit":3}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "rss_fetch.latest");
    assert_eq!(policy.contract_match, "rss_news_fetch");
}

#[test]
fn execution_failed_step_contract_accepts_command_output_evidence() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ExecutionFailedStep)
        .expect("execution failed step contract");
    let matched = MatchedContract::Semantic(contract);
    let evidence_expression = matched.evidence_expression();

    assert_eq!(matched.required_evidence(), vec!["command_output"]);
    for token in ["command_output", "field_value"] {
        assert!(
            evidence_expression.any_of.contains(&token.to_string()),
            "missing {token} in {evidence_expression:?}"
        );
    }
}

#[test]
fn service_status_allows_system_basic_info_observation() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ServiceStatus)
        .expect("service status contract");
    let matched = MatchedContract::Semantic(contract);
    let action_ref = ActionRef::parse("system_basic.info").expect("action parses");

    assert_eq!(
        matched.action_policy(&action_ref),
        ActionPolicyDecision::Allowed
    );
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
    let photo = matrix
        .contracts
        .get("photo_organization")
        .expect("photo organization transitional contract");
    assert_eq!(
        photo.migration_status,
        "transitional_capability_owned_evidence_pending"
    );
    assert_eq!(photo.migration_owner, "photo_organize.planner_capabilities");
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
    let snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    })
    .expect("trace snapshot");

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
        Some("file_names")
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
fn configured_legacy_text_observation_extractors_are_reflected_in_trace() {
    let scalar_snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..IntentOutputContract::default()
    })
    .expect("scalar trace snapshot");
    let scalar_extractors = scalar_snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .expect("scalar observation extractors");
    assert!(scalar_extractors.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("archive_basic")
            && item.get("extractor_kind").and_then(Value::as_str) == Some("text_legacy")
    }));

    let archive_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ArchiveList,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let archive_trace = action_trace_for_output_contract(&archive_contract, "archive_basic.list")
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
        Some("archive_list")
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
fn final_shape_honors_scalar_hidden_entries_and_structured_key_verdicts() {
    assert_eq!(
        final_answer_shape_for_output_contract(&IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            ..IntentOutputContract::default()
        }),
        Some(FinalAnswerShape::Scalar)
    );
    assert_eq!(
        final_answer_shape_for_output_contract(&IntentOutputContract {
            response_shape: OutputResponseShape::Free,
            semantic_kind: OutputSemanticKind::StructuredKeys,
            ..IntentOutputContract::default()
        }),
        Some(FinalAnswerShape::ValidationVerdict)
    );
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
    let trace = action_trace_for_output_contract(
        &IntentOutputContract {
            semantic_kind: OutputSemanticKind::FileNames,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        },
        "fs_basic.list_dir",
    )
    .expect("action trace should resolve");

    assert_eq!(
        trace.get("contract_match").and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        trace.get("decision").and_then(Value::as_str),
        Some("allowed")
    );
    assert_eq!(
        trace.get("final_answer_shape").and_then(Value::as_str),
        Some("name_list")
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
