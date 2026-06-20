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
fn contract_runtime_rejects_natural_language_evidence_profile() {
    let source = include_str!("../../../configs/task_contract_matrix.toml").replace(
        "evidence_profile = \"workspace_user_docs_first\"",
        "evidence_profile = \"read user setup docs first\"",
    );
    let err = parse_contract_matrix_source(&source)
        .expect_err("natural-language evidence profile should fail shape validation");

    assert!(err.contains("invalid evidence_profile"));
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
            values.contains(&"content_excerpt")
                && values.contains(&"candidates")
                && values.contains(&"count")
        }));
}

#[test]
fn generic_path_content_allows_count_entries_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "count_inventory",
            "path": "crates"
        }),
    )
    .expect("action policy");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "fs_basic.count_entries");
    assert_eq!(policy.contract_match, "generic_path_content");
}

#[test]
fn generic_path_content_allows_stat_paths_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            response_shape: OutputResponseShape::Free,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "path_batch_facts",
            "paths": ["/workspace/not_real_20260511"],
            "include_missing": true
        }),
    )
    .expect("action policy");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "fs_basic.stat_paths");
    assert_eq!(policy.contract_match, "generic_path_content");
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
fn existence_with_path_prefers_path_facts_but_allows_verifier_requested_excerpt() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Filename,
        response_shape: OutputResponseShape::Strict,
        locator_hint: "restart_once.sh".to_string(),
        ..IntentOutputContract::default()
    };

    let stat_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({"action":"stat_paths","paths":["restart_once.sh"]}),
    )
    .expect("stat policy");
    assert!(stat_policy.is_allowed(), "{stat_policy:?}");
    assert_eq!(stat_policy.action_key, "fs_basic.stat_paths");
    assert_eq!(stat_policy.contract_match, "existence_with_path");

    let read_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({"action":"read_text_range","path":"restart_once.sh","mode":"head","n":120}),
    )
    .expect("read policy");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert_eq!(read_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(read_policy.contract_match, "existence_with_path");
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
fn generated_file_path_report_allows_command_then_write_and_returns_single_path_shape() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::GeneratedFilePathReport,
        requires_content_evidence: true,
        response_shape: OutputResponseShape::Scalar,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "pwd_line_abs.txt".to_string(),
        ..IntentOutputContract::default()
    };
    let run_policy =
        action_policy_for_output_contract(Some(&contract), "run_cmd", &serde_json::json!({}))
            .expect("run policy decision");
    assert!(run_policy.is_allowed(), "{run_policy:?}");
    assert_eq!(run_policy.action_key, "run_cmd");
    assert_eq!(run_policy.contract_match, "generated_file_path_report");
    assert_eq!(run_policy.final_answer_shape, "single_path");

    let write_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"write_text","path":"pwd_line_abs.txt","content":"x"}),
    )
    .expect("write policy decision");
    assert!(write_policy.is_allowed(), "{write_policy:?}");
    assert_eq!(write_policy.action_key, "fs_basic.write_text");
    assert_eq!(write_policy.contract_match, "generated_file_path_report");
    assert_eq!(write_policy.final_answer_shape, "single_path");
}

#[test]
fn filesystem_mutation_result_allows_directory_creation_status() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "document/nl_skill_tmp".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"make_dir","path":"document/nl_skill_tmp"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.make_dir");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
    assert_eq!(policy.final_answer_shape, "lifecycle_result");
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
fn generated_file_delivery_allows_runtime_command_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "pwd_report.txt".to_string(),
            ..IntentOutputContract::default()
        }),
        "run_cmd",
        &serde_json::json!({"command":"pwd > pwd_report.txt && cat pwd_report.txt"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "run_cmd");
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
fn content_excerpt_summary_allows_health_check_field_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "health_check",
        &serde_json::json!({}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "health_check");
    assert_eq!(policy.contract_match, "content_excerpt_summary");
}

#[test]
fn content_excerpt_summary_allows_structured_field_evidence() {
    for (action, expected_action_key) in [
        (
            serde_json::json!({
                "action": "read_field",
                "path": "package.json",
                "field_path": "name"
            }),
            "config_basic.read_field",
        ),
        (
            serde_json::json!({
                "action": "read_fields",
                "path": "Cargo.toml",
                "field_paths": ["package.name", "workspace.package.version"]
            }),
            "config_basic.read_fields",
        ),
    ] {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract {
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::Path,
                locator_hint: "package.json".to_string(),
                response_shape: OutputResponseShape::Scalar,
                ..IntentOutputContract::default()
            }),
            "config_basic",
            &action,
        )
        .expect("policy decision");

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action_key);
        assert_eq!(policy.contract_match, "content_excerpt_summary");
    }
}

#[test]
fn filesystem_mutation_result_allows_archive_pack_path_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "tmp/nl_archive_case_en.zip".to_string(),
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "archive_basic",
        &serde_json::json!({
            "action": "pack",
            "source": "scripts/skill_calls",
            "archive": "tmp/nl_archive_case_en.zip",
            "format": "zip"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "archive_basic.pack");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
}

#[test]
fn filesystem_mutation_result_allows_kb_ingest_path_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "README.md".to_string(),
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "kb",
        &serde_json::json!({
            "action": "ingest",
            "namespace": "demo_docs_nl",
            "paths": ["/home/guagua/rustclaw/README.md"]
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "kb.ingest");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
}

#[test]
fn content_excerpt_summary_allows_supplemental_directory_inventory() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "UI/package.json".to_string(),
        response_shape: OutputResponseShape::OneSentence,
        ..IntentOutputContract::default()
    };

    let list_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":".","names_only":true}),
    )
    .expect("list policy");
    assert!(list_policy.is_allowed(), "{list_policy:?}");
    assert_eq!(list_policy.action_key, "fs_basic.list_dir");
    assert_eq!(list_policy.contract_match, "content_excerpt_summary");

    let find_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"find_entries","root":".","pattern":"package.json"}),
    )
    .expect("find policy");
    assert!(find_policy.is_allowed(), "{find_policy:?}");
    assert_eq!(find_policy.action_key, "fs_basic.find_entries");
    assert_eq!(find_policy.contract_match, "content_excerpt_summary");

    let count_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"count_entries","path":"crates"}),
    )
    .expect("count policy");
    assert!(count_policy.is_allowed(), "{count_policy:?}");
    assert_eq!(count_policy.action_key, "fs_basic.count_entries");
    assert_eq!(count_policy.contract_match, "content_excerpt_summary");
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
fn market_quote_allows_crypto_positions_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::MarketQuote,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "crypto",
        &serde_json::json!({"action":"positions"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "crypto.positions");
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

#[path = "contract_matrix_tests/action_policy_registry.rs"]
mod action_policy_registry;
