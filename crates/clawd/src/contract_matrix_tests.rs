use std::collections::BTreeMap;
use std::path::PathBuf;

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
fn recent_scalar_equality_allows_structured_field_extractors() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::RecentScalarEqualityCheck)
        .expect("recent scalar equality contract");
    let matched = MatchedContract::Semantic(contract);
    for action in ["config_basic.read_field", "config_basic.read_fields"] {
        let action_ref = ActionRef::parse(action).expect("action parses");
        assert_eq!(
            matched.action_policy(&action_ref),
            ActionPolicyDecision::Allowed,
            "{action} should be allowed for scalar field comparisons"
        );
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
    assert_eq!(policy.required_evidence, vec!["field_value"]);
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
        } else {
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
fn configured_legacy_text_observation_extractors_extend_default_structured_extractors() {
    let git_snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::GitCommitSubject,
        ..IntentOutputContract::default()
    })
    .expect("git trace snapshot");
    let git_extractors = git_snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .expect("git observation extractors");
    assert!(git_extractors.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("git_basic")
            && item.get("extractor_kind").and_then(Value::as_str) == Some("structured_json")
    }));
    assert!(git_extractors.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("git_basic")
            && item.get("extractor_kind").and_then(Value::as_str) == Some("text_legacy")
    }));

    let archive_snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::ArchiveList,
        ..IntentOutputContract::default()
    })
    .expect("archive trace snapshot");
    let archive_extractors = archive_snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .expect("archive observation extractors");
    assert!(archive_extractors.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("archive_basic.list")
            && item.get("extractor_kind").and_then(Value::as_str) == Some("structured_json")
    }));
    assert!(archive_extractors.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("archive_basic")
            && item.get("extractor_kind").and_then(Value::as_str) == Some("text_legacy")
    }));
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

#[test]
fn contract_runtime_fields_are_validated() {
    let err = parse_contract_matrix_source(
        r#"
schema_version = 1
matrix_version = "invalid-runtime-field"
failure_attribution = [
  "model_error",
  "schema_error",
  "code_gap",
  "contract_gap",
  "tool_gap",
  "permission_denied",
  "budget_exhausted",
  "prompt_budget_error",
  "delivery_error",
  "provider_error",
]

[trace_policy]
evidence_storage = "redacted_excerpt_hash"
provider_evidence_view = "provider_safe_redacted"
raw_excerpt_policy = "no_full_raw_excerpt"
max_items = 24
max_excerpt_chars = 240

[contracts.none]
semantic_kind = "none"
operation = "unknown"
target_object = "unknown"
delivery_shape = "summary"
policy_mode = "maybe"
allowed_actions = []
preferred_actions = []
forbidden_actions = []
required_evidence = []
final_answer_shape = "free"
none_passthrough = true
failure_policy = "no_retry"
"#,
    )
    .expect_err("invalid runtime field should fail shape validation");

    assert!(err.contains("invalid policy_mode"));
}

#[test]
fn configured_observation_extractors_must_exist_in_registry() {
    let source = format!(
            "{}\n[[contracts.service_status.observation_extractors]]\nsource = \"run_cmd\"\nextractor_kind = \"structured_json\"\n",
            include_str!("../../../configs/task_contract_matrix.toml")
        );
    let err = parse_contract_matrix_source(&source)
        .expect_err("unregistered explicit extractor should fail validation");

    assert!(err.contains(
            "observation_extractor source `run_cmd` with extractor_kind `structured_json` is not declared in the evidence extractor registry"
        ));
}

#[test]
fn runtime_contract_snapshot_binds_matrix_and_compact_prompt_block() {
    let snapshot = runtime_contract_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    })
    .expect("runtime contract snapshot");

    assert_eq!(
        snapshot
            .get("matrix")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("bundled:configs/task_contract_matrix.toml")
    );
    assert!(snapshot
        .get("matrix")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(
        snapshot
            .get("registry")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("bundled:configs/skills_registry.toml")
    );
    assert!(snapshot
        .get("registry")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(
        snapshot
            .get("prompt_layer")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("bundled:prompts/layers/manifest.toml")
    );
    assert!(snapshot
        .get("prompt_layer")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert!(snapshot
        .get("compact_contract_block")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("contract_match"))
            .and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("final_answer_shape"))
            .and_then(Value::as_str),
        Some("name_list")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("final_answer_shape_class"))
            .and_then(Value::as_str),
        Some("strict_list")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("coarse_response_shape"))
            .and_then(Value::as_str),
        Some("strict")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("allows_model_language"))
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn contract_matrix_final_answer_shapes_are_typed() {
    let matrix = load_workspace_matrix();
    let configured = matrix
        .contracts
        .values()
        .map(|contract| contract.final_answer_shape.as_str())
        .chain(
            matrix
                .generic_profiles
                .iter()
                .map(|profile| profile.final_answer_shape.as_str()),
        )
        .collect::<BTreeSet<_>>();
    let typed = FinalAnswerShape::ALL
        .iter()
        .map(|shape| shape.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(configured, typed);
    for shape in configured {
        assert_eq!(
            FinalAnswerShape::parse(shape).map(FinalAnswerShape::as_str),
            Some(shape)
        );
    }
}

#[test]
fn final_answer_shape_classes_are_total_and_runtime_mapped() {
    let mut classes = BTreeSet::new();
    for shape in FinalAnswerShape::ALL {
        classes.insert(shape.class().as_str());
        assert_eq!(
            shape.coarse_response_shape(),
            shape.class().coarse_response_shape()
        );
    }

    assert!(classes.contains("delivery_artifact"));
    assert!(classes.contains("scalar_value"));
    assert!(classes.contains("single_path"));
    assert!(classes.contains("strict_list"));
    assert!(classes.contains("table"));
    assert!(classes.contains("verdict"));
    assert!(classes.contains("grounded_summary"));
    assert!(classes.contains("freeform"));

    assert_eq!(
        FinalAnswerShape::DeliveryTokenOrPath.coarse_response_shape(),
        OutputResponseShape::FileToken
    );
    assert_eq!(
        FinalAnswerShape::Scalar.coarse_response_shape(),
        OutputResponseShape::Scalar
    );
    assert_eq!(
        FinalAnswerShape::SinglePath.coarse_response_shape(),
        OutputResponseShape::Scalar
    );
    assert_eq!(
        FinalAnswerShape::NameList.coarse_response_shape(),
        OutputResponseShape::Strict
    );
    assert_eq!(
        FinalAnswerShape::TableListing.coarse_response_shape(),
        OutputResponseShape::Strict
    );
    assert_eq!(
        FinalAnswerShape::ValidationVerdict.coarse_response_shape(),
        OutputResponseShape::OneSentence
    );
    assert_eq!(
        FinalAnswerShape::SummaryWithEvidence.coarse_response_shape(),
        OutputResponseShape::Free
    );
    assert!(!FinalAnswerShape::NameList.allows_model_language());
    assert!(FinalAnswerShape::SummaryWithEvidence.allows_model_language());
}

#[test]
fn delivery_artifact_contracts_declare_file_artifact_kind() {
    let matrix = load_workspace_matrix();

    for (key, contract) in &matrix.contracts {
        let shape = FinalAnswerShape::parse(&contract.final_answer_shape)
            .expect("contract final_answer_shape should be typed");
        if shape.class() == FinalAnswerShapeClass::DeliveryArtifact {
            assert_eq!(
                contract.artifact_kind(),
                "file",
                "delivery artifact contract `{key}` must not default to text"
            );
            assert_eq!(
                contract.channel_visibility(),
                "user_visible",
                "delivery artifact contract `{key}` must be user visible"
            );
        }
        if normalize_action_token(&contract.delivery_shape) == "file" {
            assert_eq!(
                contract.artifact_kind(),
                "file",
                "file delivery-shape contract `{key}` must declare artifact_kind=file"
            );
        }
    }

    for profile in &matrix.generic_profiles {
        let shape = FinalAnswerShape::parse(&profile.final_answer_shape)
            .expect("profile final_answer_shape should be typed");
        if shape.class() == FinalAnswerShapeClass::DeliveryArtifact {
            assert_eq!(
                profile.artifact_kind(),
                "file",
                "delivery artifact generic profile `{}` must not default to text",
                profile.name
            );
            assert_eq!(
                profile.channel_visibility(),
                "user_visible",
                "delivery artifact generic profile `{}` must be user visible",
                profile.name
            );
        }
    }
}

#[test]
fn contract_matrix_evidence_tokens_are_typed() {
    let matrix = load_workspace_matrix();
    let mut configured = BTreeSet::new();

    for contract in matrix.contracts.values() {
        configured.extend(
            contract
                .normalized_required_evidence()
                .into_iter()
                .filter(|field| !field.is_empty()),
        );
        configured.extend(evidence_expression_tokens(&contract.evidence_expression));
    }
    for profile in &matrix.generic_profiles {
        configured.extend(
            profile
                .normalized_required_evidence()
                .into_iter()
                .filter(|field| !field.is_empty()),
        );
        configured.extend(evidence_expression_tokens(&profile.evidence_expression));
    }

    let typed = EvidenceToken::ALL
        .iter()
        .map(|token| token.as_str().to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(configured, typed);
    for token in configured {
        assert_eq!(
            EvidenceToken::parse(&token).map(EvidenceToken::as_str),
            Some(token.as_str())
        );
    }
}

#[test]
fn bundled_contract_matrix_renders_prompt_line() {
    let line = compact_prompt_line_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    })
    .expect("contract prompt line");

    assert!(line.contains("contract_matrix"));
    assert!(line.contains("match=file_names"));
    assert!(line.contains("required_evidence=candidates"));
}

#[test]
fn contract_matrix_covers_all_output_semantic_kinds() {
    let matrix = load_workspace_matrix();

    let missing = OutputSemanticKind::ALL
        .iter()
        .filter(|kind| matrix.semantic_contract(**kind).is_none())
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "missing semantic contracts: {missing:?}"
    );
}

#[test]
fn contract_matrix_evidence_matches_task_contract_defaults() {
    let matrix = load_workspace_matrix();

    for kind in OutputSemanticKind::ALL {
        let output_contract = IntentOutputContract {
            semantic_kind: *kind,
            ..IntentOutputContract::default()
        };
        let expected = fallback_required_evidence_fields_for_output_contract(&output_contract);
        let actual = matrix
            .semantic_contract(*kind)
            .expect("semantic contract")
            .normalized_required_evidence();

        assert_eq!(
            actual,
            expected,
            "evidence mismatch for semantic `{}`",
            kind.as_str()
        );
    }
}

#[test]
fn route_specific_evidence_augments_matrix_base_contract() {
    let required = required_evidence_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::QuantityComparison,
        locator_kind: OutputLocatorKind::Filename,
        ..IntentOutputContract::default()
    })
    .expect("required evidence");

    assert_eq!(
        required,
        vec!["exists", "field_value", "kind", "size_bytes"]
    );
}

#[test]
fn quantity_comparison_allows_directory_count_size_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::QuantityComparison,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "count_entries",
            "path": "target",
            "recursive": true
        }),
    )
    .expect("action policy");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "fs_basic.count_entries");
}

#[test]
fn generic_profile_matches_untyped_path_content_contract() {
    let matrix = load_workspace_matrix();
    let matched = matrix
        .match_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            response_shape: OutputResponseShape::Free,
            ..IntentOutputContract::default()
        })
        .expect("generic profile match");

    assert_eq!(matched.required_evidence(), vec!["content_excerpt", "path"]);
    assert_eq!(matched.final_answer_shape(), "summary_with_evidence");
    let evidence_expression = matched
        .evidence_expression()
        .to_trace_json(&matched.required_evidence());
    assert_eq!(
        evidence_expression
            .pointer("/all_of/0")
            .and_then(Value::as_str),
        Some("path")
    );
    assert!(evidence_expression
        .get("any_of")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            let values = items.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            values.contains(&"content_excerpt") && values.contains(&"candidates")
        }));
}

#[test]
fn generic_delivery_takes_precedence_for_untyped_file_token_delivery() {
    let matrix = load_workspace_matrix();
    let matched = matrix
        .match_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            locator_kind: OutputLocatorKind::Path,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        })
        .expect("generic delivery profile match");

    assert_eq!(matched.match_name(), "generic_delivery");
    assert_eq!(matched.final_answer_shape(), "delivery_token_or_path");
}

#[test]
fn generic_delivery_allows_structured_missing_file_probes() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        response_shape: OutputResponseShape::FileToken,
        ..IntentOutputContract::default()
    };

    let stat_policy = action_policy_for_output_contract(
        Some(&contract),
        "system_basic",
        &json!({"action":"path_batch_facts", "paths":["definitely_missing.txt"]}),
    )
    .expect("stat policy");
    assert!(stat_policy.is_allowed(), "{stat_policy:?}");
    assert_eq!(stat_policy.action_key, "fs_basic.stat_paths");
    assert_eq!(stat_policy.contract_match, "generic_delivery");

    let find_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_search",
        &json!({"action":"find_name", "pattern":"definitely_missing.txt"}),
    )
    .expect("find policy");
    assert!(find_policy.is_allowed(), "{find_policy:?}");
    assert_eq!(find_policy.action_key, "fs_basic.find_entries");
    assert_eq!(find_policy.contract_match, "generic_delivery");
}

#[test]
fn quantity_comparison_allows_readonly_content_followup() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::QuantityComparison,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };

    let read_policy = action_policy_for_output_contract(
            Some(&contract),
            "fs_basic",
            &json!({"action":"read_text_range", "path":"prompts/schemas/intent_normalizer.schema.json"}),
        )
        .expect("read policy");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert_eq!(read_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(read_policy.contract_match, "quantity_comparison");

    let config_policy = action_policy_for_output_contract(
            Some(&contract),
            "config_basic",
            &json!({"action":"read_fields", "path":"prompts/schemas/intent_normalizer.schema.json", "field_paths":["title","description"]}),
        )
        .expect("config field policy");
    assert!(config_policy.is_allowed(), "{config_policy:?}");
    assert_eq!(config_policy.action_key, "config_basic.read_fields");
    assert_eq!(config_policy.contract_match, "quantity_comparison");
}

#[test]
fn generated_file_delivery_allows_parent_directory_creation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"make_dir","path":"tmp"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.make_dir");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_existing_file_path_facts() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"stat_paths","paths":["README.md"]}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.stat_paths");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_existing_file_content_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "README.md".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"read_text_range","path":"README.md","mode":"head","n":30}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_directory_inventory_for_existing_selection() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"document","names_only":true}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.list_dir");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_audio_synthesis_file_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "audio_synthesize",
        &serde_json::json!({
            "text": "RustClaw skill test passed",
            "output_path": "document/skill_audio_smoke.mp3"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "audio_synthesize");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_image_generation_file_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "image_generate",
        &serde_json::json!({
            "prompt": "minimal RustClaw smoke test card",
            "output_path": "document/skill_generate_smoke.png"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "image_generate");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_image_edit_file_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "image_edit",
        &serde_json::json!({
            "action": "restyle",
            "instruction": "pixel art style",
            "image": {"url": "https://example.test/source.png"},
            "output_path": "document/rust_icon_pixel.png"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "image_edit.restyle");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn content_excerpt_summary_allows_log_analyze_for_log_paths() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "logs".to_string(),
            ..IntentOutputContract::default()
        }),
        "log_analyze",
        &serde_json::json!({"action":"analyze","path":"logs"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "log_analyze.analyze");
    assert_eq!(policy.contract_match, "content_excerpt_summary");
}

#[test]
fn web_page_summary_allows_browser_open_extract_for_url() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WebPageSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Url,
            locator_hint: "https://example.com".to_string(),
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "browser_web",
        &serde_json::json!({
            "action": "open_extract",
            "url": "https://example.com"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "browser_web.open_extract");
    assert_eq!(policy.contract_match, "web_page_summary");
}

#[test]
fn web_search_summary_allows_web_search_extract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WebSearchSummary,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::Strict,
            ..IntentOutputContract::default()
        }),
        "web_search_extract",
        &serde_json::json!({
            "action": "search_extract",
            "query": "rust async tutorial",
            "top_k": 3
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "web_search_extract.search_extract");
    assert_eq!(policy.contract_match, "web_search_summary");
}

#[test]
fn web_search_summary_allows_followup_browser_extract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WebSearchSummary,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "browser_web",
        &serde_json::json!({
            "action": "open_extract",
            "urls": ["https://example.com"]
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "browser_web.open_extract");
    assert_eq!(policy.contract_match, "web_search_summary");
}

#[test]
fn weather_query_allows_weather_query_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WeatherQuery,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "weather",
        &serde_json::json!({"action":"query","city":"Beijing"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "weather.query");
    assert_eq!(policy.contract_match, "weather_query");
    assert_eq!(policy.required_evidence, vec!["content_excerpt"]);
}

#[test]
fn market_quote_allows_stock_quote_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::MarketQuote,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "stock",
        &serde_json::json!({"symbol":"600519"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "stock");
    assert_eq!(policy.contract_match, "market_quote");
    assert_eq!(policy.required_evidence, vec!["content_excerpt"]);
}

#[test]
fn market_quote_allows_crypto_quote_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::MarketQuote,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "crypto",
        &serde_json::json!({"action":"quote","symbol":"BTCUSDT"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "crypto.quote");
    assert_eq!(policy.contract_match, "market_quote");
}

#[test]
fn image_understanding_allows_image_vision_describe_with_url_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ImageUnderstanding,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::Url,
            locator_hint: "https://example.com/image.png".to_string(),
            ..IntentOutputContract::default()
        }),
        "image_vision",
        &serde_json::json!({"action":"describe","image":"https://example.com/image.png"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "image_vision.describe");
    assert_eq!(policy.contract_match, "image_understanding");
    assert_eq!(policy.required_evidence, vec!["content_excerpt"]);
}

#[test]
fn image_understanding_allows_image_vision_analyze_alias() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ImageUnderstanding,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::Url,
            locator_hint: "https://example.com/image.png".to_string(),
            ..IntentOutputContract::default()
        }),
        "image_vision",
        &serde_json::json!({"action":"analyze","image":"https://example.com/image.png"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "image_vision.analyze");
    assert_eq!(policy.contract_match, "image_understanding");
}

#[test]
fn publishing_preview_allows_x_preview_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::PublishingPreview,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "x",
        &serde_json::json!({"action":"preview","text":"RustClaw release notes","dry_run":true}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "x.preview");
    assert_eq!(policy.contract_match, "publishing_preview");
    assert_eq!(policy.required_evidence, vec!["field_value"]);
}

#[test]
fn content_excerpt_with_summary_contract_has_parsed_final_shape() {
    let output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptWithSummary,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "logs/model_io.log".to_string(),
        ..IntentOutputContract::default()
    };

    let shape =
        final_answer_shape_for_output_contract(&output_contract).expect("final answer shape");

    assert_eq!(shape, FinalAnswerShape::ExcerptPlusSummary);
    assert_eq!(shape.as_str(), "excerpt_plus_summary");
    assert_eq!(shape.class(), FinalAnswerShapeClass::GroundedSummary);
}

#[test]
fn excerpt_kind_judgment_allows_directory_listing_context() {
    for (capability, args, expected_action) in [
        (
            "fs_basic",
            serde_json::json!({"action":"list_dir","path":"docs","names_only":true}),
            "fs_basic.list_dir",
        ),
        (
            "system_basic",
            serde_json::json!({"action":"inventory_dir","path":"docs","names_only":true}),
            "fs_basic.list_dir",
        ),
    ] {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract {
                semantic_kind: OutputSemanticKind::ExcerptKindJudgment,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::Path,
                locator_hint: "docs/release_checklist.md".to_string(),
                ..IntentOutputContract::default()
            }),
            capability,
            &args,
        )
        .expect("policy decision");

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "excerpt_kind_judgment");
    }
}

#[test]
fn directory_purpose_summary_allows_log_analyze_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "logs".to_string(),
            ..IntentOutputContract::default()
        }),
        "log_analyze",
        &serde_json::json!({"path":"logs"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "log_analyze");
    assert_eq!(policy.contract_match, "directory_purpose_summary");
}

#[test]
fn directory_purpose_summary_allows_structured_field_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "extract_field",
            "path": "UI/package.json",
            "field_path": "name"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.read_field");
    assert_eq!(policy.contract_match, "directory_purpose_summary");
}

#[test]
fn recent_artifacts_judgment_allows_bounded_content_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "docs".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "docs/config_basic_contract.md",
            "mode": "head",
            "n": 40
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
    assert_eq!(policy.contract_match, "recent_artifacts_judgment");
}

#[test]
fn workspace_project_summary_requires_bounded_content_after_tree_discovery() {
    let tree_policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({"action":"tree_summary","path":"/workspace","max_depth":1}),
    )
    .expect("policy decision");
    assert!(tree_policy.is_allowed(), "{tree_policy:?}");
    assert_eq!(tree_policy.action_key, "system_basic.tree_summary");
    assert_eq!(tree_policy.contract_match, "workspace_project_summary");

    let read_policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"read_text_range","path":"README.md","mode":"head","n":80}),
    )
    .expect("policy decision");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert_eq!(read_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(read_policy.contract_match, "workspace_project_summary");

    let list_policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"/workspace","names_only":true}),
    )
    .expect("policy decision");
    assert!(!list_policy.is_allowed(), "{list_policy:?}");
    assert_eq!(list_policy.action_key, "fs_basic.list_dir");
    assert_eq!(list_policy.contract_match, "workspace_project_summary");
}

#[test]
fn generic_delivery_allows_directory_listing_for_selection() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "document".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"document","files_only":true}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.list_dir");
    assert_eq!(policy.contract_match, "generic_delivery");
}

#[test]
fn semantic_none_rejects_forbidden_action() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .match_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        })
        .expect("matched contract");
    let action = ActionRef::parse("run_cmd").expect("action ref");

    assert_eq!(
        contract.action_policy(&action),
        ActionPolicyDecision::RejectedForbidden
    );
}

#[test]
fn action_policy_blocks_disallowed_structured_action_for_semantic_contract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FileNames,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "run_cmd",
        &serde_json::json!({"command":"ls"}),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::RejectedNotAllowed);
    assert_eq!(policy.contract_match, "file_names");
    assert_eq!(policy.required_evidence, vec!["candidates"]);
}

#[test]
fn action_policy_allows_process_snapshot_for_raw_command_output_contract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "process_basic",
        &serde_json::json!({
            "action": "ps",
            "limit": 10,
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "process_basic.ps");
    assert_eq!(policy.contract_match, "raw_command_output");
}

#[test]
fn action_policy_allows_http_observation_for_raw_command_output_contract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "http_basic",
        &serde_json::json!({
            "action": "get",
            "url": "http://127.0.0.1:8787/v1/health",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "http_basic.get");
    assert_eq!(policy.contract_match, "raw_command_output");
    assert!(policy
        .evidence_expression
        .any_of
        .contains(&"command_output".to_string()));
}

#[test]
fn action_policy_allows_safe_file_read_equivalent_for_raw_command_output_contract() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    let fs_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "logs/clawd.log",
            "mode": "tail",
            "n": 20,
        }),
    )
    .expect("fs policy decision");
    assert_eq!(fs_policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(fs_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(fs_policy.contract_match, "raw_command_output");
    assert!(fs_policy
        .evidence_expression
        .any_of
        .contains(&"content_excerpt".to_string()));

    let system_policy = action_policy_for_output_contract(
        Some(&contract),
        "system_basic",
        &serde_json::json!({
            "action": "read_range",
            "path": "logs/clawd.log",
            "mode": "tail",
            "n": 20,
        }),
    )
    .expect("system policy decision");
    assert_eq!(system_policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(system_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(system_policy.contract_match, "raw_command_output");
}

#[test]
fn action_policy_allows_runtime_equivalent_for_virtual_config_validation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.contract_match, "config_validation");
}

#[test]
fn action_policy_allows_runtime_equivalent_for_virtual_config_guard() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "config_edit",
        &serde_json::json!({
            "action": "guard_config",
            "path": "configs/app_config.toml",
            "format": "toml",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "config_basic.guard_rustclaw_config");
    assert_eq!(policy.contract_match, "config_validation");
}

#[test]
fn config_mutation_contract_allows_plan_apply_validate_and_read_back() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ConfigMutation,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        ..IntentOutputContract::default()
    };

    for action in [
        "plan_config_change",
        "apply_config_change",
        "validate_config",
        "read_back",
    ] {
        let policy = action_policy_for_output_contract(
            Some(&contract),
            "config_edit",
            &serde_json::json!({
                "action": action,
                "path": "configs/config.toml",
                "field_path": "skills.skill_switches.example",
                "value": true,
            }),
        )
        .expect("policy decision");

        assert_eq!(policy.decision, ActionPolicyDecision::Allowed, "{action}");
        assert_eq!(policy.contract_match, "config_mutation");
    }
}

#[test]
fn action_policy_skips_unstructured_none_contracts() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract::default()),
        "run_cmd",
        &serde_json::json!({"command":"echo ok"}),
    );

    assert!(policy.is_none());
}

#[test]
fn arg_policy_defers_unresolved_template_targets() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "{{s1.path}}"
        }),
    )
    .expect("arg policy decision");

    assert_eq!(policy.decision, ArgPolicyDecision::DeferredTemplateArg);
    assert!(policy.missing_target_args.is_empty());
    assert_eq!(policy.deferred_target_args, vec!["path"]);
}

#[test]
fn arg_policy_rejects_missing_bound_target_after_resolution() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "start_line": 1,
            "end_line": 20
        }),
    )
    .expect("arg policy decision");

    assert_eq!(policy.decision, ArgPolicyDecision::MissingTargetBinding);
    assert_eq!(policy.missing_target_args, vec!["path"]);
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
}

#[test]
fn arg_policy_allows_concrete_bound_target() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "/tmp/readme.md"
        }),
    )
    .expect("arg policy decision");

    assert!(policy.is_allowed());
    assert!(policy.missing_target_args.is_empty());
    assert!(policy.deferred_target_args.is_empty());
}

#[test]
fn arg_policy_uses_virtual_equivalent_target_groups() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("arg policy decision");

    assert!(policy.is_allowed());
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.expected_target_args, vec!["path"]);
}

#[test]
fn arg_policy_uses_virtual_guard_equivalent_target_groups() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "config_edit",
        &serde_json::json!({
            "action": "guard_config",
            "path": "configs/app_config.toml",
        }),
    )
    .expect("arg policy decision");

    assert!(policy.is_allowed());
    assert_eq!(policy.action_key, "config_basic.guard_rustclaw_config");
    assert_eq!(policy.expected_target_args, vec!["path"]);
}

#[test]
fn action_ref_prefers_structured_action_from_args() {
    let action = ActionRef::from_skill_args("fs-basic", &serde_json::json!({"action":"list_dir"}))
        .expect("action ref");

    assert_eq!(action.as_key(), "fs_basic.list_dir");
}

#[test]
fn contract_matrix_references_registered_skills() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

    let unknown = matrix.unknown_matrix_skills(&registry);

    assert!(unknown.is_empty(), "unknown matrix skills: {unknown:?}");
}

#[test]
fn contract_matrix_action_refs_are_declared_in_registry() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

    let unknown = matrix.unknown_matrix_action_refs(&registry);

    assert!(
        unknown.is_empty(),
        "unknown matrix action refs: {unknown:?}"
    );
}

#[test]
fn contract_matrix_action_refs_have_registry_schemas() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    let mut missing = Vec::new();

    for token in matrix.all_action_tokens() {
        let Some(action_ref) = ActionRef::parse(&token) else {
            continue;
        };
        let Some(skill) = registry.resolve_canonical(&action_ref.skill) else {
            continue;
        };
        let Some(manifest) = registry.manifest(skill) else {
            continue;
        };
        if manifest.input_schema.is_none() {
            missing.push(format!("{}.input_schema", action_ref.skill));
        }
        if manifest.output_schema.is_none() {
            missing.push(format!("{}.output_schema", action_ref.skill));
        }
    }
    missing.sort();
    missing.dedup();

    assert!(missing.is_empty(), "missing registry schemas: {missing:?}");
}

#[test]
fn legacy_virtual_tool_canonicalizations_are_covered_by_matrix_action_policy() {
    let cases = [
        (
            OutputSemanticKind::ExistenceWithPath,
            "system_basic",
            json!({"action":"path_batch_facts", "paths":["README.md"]}),
            "fs_basic.stat_paths",
        ),
        (
            OutputSemanticKind::FileNames,
            "system_basic",
            json!({"action":"inventory_dir", "path":"scripts"}),
            "fs_basic.list_dir",
        ),
        (
            OutputSemanticKind::ScalarCount,
            "system_basic",
            json!({"action":"count_inventory", "path":"scripts"}),
            "fs_basic.count_entries",
        ),
        (
            OutputSemanticKind::ContentExcerptSummary,
            "system_basic",
            json!({"action":"read_range", "path":"README.md", "mode":"head", "n":5}),
            "fs_basic.read_text_range",
        ),
        (
            OutputSemanticKind::QuantityComparison,
            "system_basic",
            json!({"action":"compare_paths", "paths":["Cargo.toml", "README.md"]}),
            "fs_basic.compare_paths",
        ),
        (
            OutputSemanticKind::QuantityComparison,
            "system_basic",
            json!({"action":"count_inventory", "path":"target", "recursive":true}),
            "fs_basic.count_entries",
        ),
        (
            OutputSemanticKind::ConfigValidation,
            "system_basic",
            json!({"action":"validate_structured", "path":"configs/config.toml", "format":"toml"}),
            "config_basic.validate",
        ),
        (
            OutputSemanticKind::FilePaths,
            "fs_search",
            json!({"action":"find_ext", "root":"scripts", "ext":"sh"}),
            "fs_basic.find_entries",
        ),
        (
            OutputSemanticKind::ContentPresenceCheck,
            "fs_search",
            json!({"action":"grep_text", "root":".", "query":"FirstLayerDecision"}),
            "fs_basic.grep_text",
        ),
        (
            OutputSemanticKind::ConfigRiskAssessment,
            "config_guard",
            json!({"path":"configs/config.toml"}),
            "config_guard",
        ),
        (
            OutputSemanticKind::ContentExcerptSummary,
            "read_file",
            json!({"path":"README.md"}),
            "fs_basic.read_text_range",
        ),
        (
            OutputSemanticKind::FileNames,
            "list_dir",
            json!({"path":"scripts"}),
            "fs_basic.list_dir",
        ),
        (
            OutputSemanticKind::GeneratedFileDelivery,
            "write_file",
            json!({"path":"tmp/out.txt", "content":"ok"}),
            "fs_basic.write_text",
        ),
    ];

    for (semantic_kind, skill, args, expected_action_key) in cases {
        let route = IntentOutputContract {
            semantic_kind,
            ..IntentOutputContract::default()
        };
        let policy = action_policy_for_output_contract(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("missing policy for {skill} -> {expected_action_key}"));
        assert!(
            policy.is_allowed(),
            "legacy {skill} should be allowed as {expected_action_key}, got {:?}",
            policy.decision
        );
        assert_eq!(policy.action_key, expected_action_key);
    }
}

#[test]
fn bundled_matrix_observation_sources_have_extractor_registry_refs() {
    let matrix = load_workspace_matrix();
    let mut missing = Vec::new();

    for (name, contract) in &matrix.contracts {
        for extractor in contract.observation_extractors() {
            if crate::task_journal::evidence_extractor_registry_trace(
                &extractor.source,
                &extractor.extractor_kind,
            )
            .is_none()
            {
                missing.push(format!(
                    "contract `{name}` observation_source `{}` extractor_kind `{}`",
                    extractor.source, extractor.extractor_kind
                ));
            }
        }
    }
    for profile in &matrix.generic_profiles {
        for extractor in profile.observation_extractors() {
            if crate::task_journal::evidence_extractor_registry_trace(
                &extractor.source,
                &extractor.extractor_kind,
            )
            .is_none()
            {
                missing.push(format!(
                    "generic profile `{}` observation_source `{}` extractor_kind `{}`",
                    profile.name, extractor.source, extractor.extractor_kind
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "missing observation extractor registry refs: {missing:?}"
    );
}

#[test]
fn contract_matrix_external_observation_sources_are_admitted() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

    let errors = matrix.external_observation_admission_errors(&registry);

    assert!(
        errors.is_empty(),
        "external observation sources need matrix admission: {errors:?}"
    );
}

#[test]
fn external_observation_source_requires_matrix_admission() {
    let matrix = ContractMatrix {
        generic_profiles: vec![GenericProfile {
            name: "external_scalar".to_string(),
            required_evidence: vec!["field_value".to_string()],
            observation_sources: vec!["demo_skill.ping".to_string()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let not_admitted = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = false, declared_actions = [], evidence_sources = [], required_extra_fields = [], extractor_kind = "structured_json", admission_version = "external-v1" }
"#,
    );

    let errors = matrix.external_observation_admission_errors(&not_admitted);

    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("demo_skill.ping"));
    assert!(errors[0].contains("matrix_admission.eligible=true"));

    let action_mismatch = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = true, declared_actions = ["other"], evidence_sources = ["structured_json"], required_extra_fields = ["extra.message"], extractor_kind = "structured_json", admission_version = "external-v1" }
"#,
    );
    let errors = matrix.external_observation_admission_errors(&action_mismatch);
    assert_eq!(errors.len(), 1);

    let admitted = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = true, declared_actions = ["ping"], evidence_sources = ["structured_json"], required_extra_fields = ["extra.message"], extractor_kind = "structured_json", admission_version = "external-v1" }
"#,
    );
    assert!(matrix
        .external_observation_admission_errors(&admitted)
        .is_empty());

    let text_legacy_matrix = ContractMatrix {
        generic_profiles: vec![GenericProfile {
            name: "external_scalar".to_string(),
            required_evidence: vec!["field_value".to_string()],
            observation_sources: vec!["demo_skill.ping".to_string()],
            observation_extractors: vec![ObservationExtractor {
                source: "demo_skill.ping".to_string(),
                extractor_kind: "text_legacy".to_string(),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let errors = text_legacy_matrix.external_observation_admission_errors(&admitted);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("text_legacy extractor"));

    let text_legacy_admitted = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = true, declared_actions = ["ping"], evidence_sources = ["text_legacy"], required_extra_fields = ["extra.message"], extractor_kind = "text_legacy", admission_version = "external-v1" }
"#,
    );
    assert!(text_legacy_matrix
        .external_observation_admission_errors(&text_legacy_admitted)
        .is_empty());
}

#[test]
fn contract_matrix_main_contracts_do_not_reference_backing_tools() {
    let matrix = load_workspace_matrix();

    let backing_refs = matrix.backing_tool_refs_in_main_contracts();

    assert!(
        backing_refs.is_empty(),
        "matrix should use planner-facing actions, not backing tools: {backing_refs:?}"
    );
}

#[test]
fn registry_action_index_contains_skill_level_and_action_level_refs() {
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    let refs = available_action_refs_from_registry(&registry);

    assert!(refs.contains("fs_basic"));
    assert!(refs.contains("fs_basic.list_dir"));
    assert!(refs.contains("archive_basic.pack"));
}

#[test]
fn matrix_generated_cases_cover_at_least_100_unique_contract_paths() {
    let matrix = load_workspace_matrix();
    let cases = generated_contract_cases(&matrix, 100);

    let mut ids = BTreeSet::new();
    let mut semantic_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut generic_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut decisions = BTreeSet::new();

    for case in &cases {
        assert!(
            ids.insert(case.id.as_str()),
            "duplicate case id: {}",
            case.id
        );

        match &case.matched {
            GeneratedContractMatch::Semantic(kind) => {
                *semantic_counts.entry(kind.as_str()).or_default() += 1;
            }
            GeneratedContractMatch::Generic(name) => {
                *generic_counts.entry(name.clone()).or_default() += 1;
            }
        }

        let matched = matched_for_generated_case(&matrix, case);
        assert_eq!(
            case.expected_required_evidence,
            matched.required_evidence(),
            "required evidence drift in generated case {}",
            case.id
        );
        assert_eq!(
            case.expected_final_answer_shape,
            matched.final_answer_shape(),
            "final answer shape drift in generated case {}",
            case.id
        );

        if let Some(action) = &case.action {
            let expected = case
                .expected_decision
                .expect("action case has expected decision");
            let actual = matched.action_policy(action);
            assert_eq!(
                actual, expected,
                "action decision drift in generated case {}",
                case.id
            );
            decisions.insert(actual.as_str());
        }
    }

    assert!(
        OutputSemanticKind::ALL
            .iter()
            .all(|kind| semantic_counts.contains_key(kind.as_str())),
        "generated cases must cover every semantic kind"
    );
    assert!(
        matrix
            .generic_profiles
            .iter()
            .all(|profile| generic_counts.contains_key(&profile.name)),
        "generated cases must cover every generic profile"
    );
    assert!(
        decisions.contains(ActionPolicyDecision::Allowed.as_str()),
        "generated cases must include allowed action decisions"
    );
    assert!(
        decisions.contains(ActionPolicyDecision::RejectedForbidden.as_str()),
        "generated cases must include forbidden action decisions"
    );
    assert!(
        decisions.contains(ActionPolicyDecision::RejectedNotAllowed.as_str()),
        "generated cases must include not-allowed action decisions"
    );
}
