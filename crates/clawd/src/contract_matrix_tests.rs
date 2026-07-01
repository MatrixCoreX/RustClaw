use std::{collections::BTreeMap, path::PathBuf};

use super::*;
use crate::task_contract::fallback_required_evidence_fields_for_output_contract;
use crate::{
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, ResumeBehavior, RiskCeiling,
    RouteResult, ScheduleKind,
};
#[path = "contract_matrix_recent_artifacts_tests.rs"]
mod recent_artifacts_tests;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn load_workspace_matrix() -> ContractMatrix {
    ContractMatrix::load_from_workspace(&workspace_root()).expect("load contract matrix")
}

fn route_with_machine_capability_ref(capability_ref: &str) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: capability_ref.to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: capability_ref.to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            semantic_kind: OutputSemanticKind::None,
            ..IntentOutputContract::default()
        },
    }
}

#[test]
fn route_capability_ref_allows_config_archive_policy_without_semantic_kind() {
    for (capability_ref, skill, args, expected_action, expected_evidence) in [
        (
            "capability_ref=config.validate",
            "config_basic",
            serde_json::json!({"action":"validate","path":"configs/config.toml"}),
            "config_basic.validate",
            vec!["valid"],
        ),
        (
            "capability_ref=archive.pack",
            "archive_basic",
            serde_json::json!({"action":"pack","source":"tmp/report","archive":"tmp/report.zip"}),
            "archive_basic.pack",
            vec!["path"],
        ),
        (
            "capability_ref=archive.unpack",
            "archive_basic",
            serde_json::json!({"action":"unpack","archive":"tmp/report.zip","dest":"tmp/report"}),
            "archive_basic.unpack",
            vec!["path"],
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
        assert_eq!(policy.required_evidence, expected_evidence);
    }
}

#[test]
fn route_capability_ref_overrides_bridge_semantic_policy_match() {
    for (semantic_kind, capability_ref, skill, args) in [
        (
            OutputSemanticKind::ConfigValidation,
            "capability_ref=config.validate",
            "config_basic",
            serde_json::json!({"action":"validate","path":"configs/config.toml"}),
        ),
        (
            OutputSemanticKind::ArchivePack,
            "capability_ref=archive.pack",
            "archive_basic",
            serde_json::json!({"action":"pack","source":"tmp/report","archive":"tmp/report.zip"}),
        ),
    ] {
        let mut route = route_with_machine_capability_ref(capability_ref);
        route.output_contract.semantic_kind = semantic_kind;

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {capability_ref}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.contract_match, "capability_ref");
        assert_eq!(policy.contract_repair_source, "capability_ref_route_policy");
    }
}

#[test]
fn route_policy_does_not_allow_config_action_without_capability_ref() {
    let mut route = route_with_machine_capability_ref("machine_context=no_capability_ref");
    route.route_reason.clear();
    route.resolved_intent.clear();

    let policy = action_policy_for_route(
        Some(&route),
        "config_basic",
        &serde_json::json!({"action":"validate","path":"configs/config.toml"}),
    )
    .expect("generic policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::RejectedNotAllowed);
    assert_ne!(policy.contract_match, "capability_ref");
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
    let output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ArchiveList,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };
    for (skill, args, expected_action) in [
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
            .expect("archive list policy decision");
        assert!(policy.is_allowed(), "{policy:?}");
        assert!(policy.action_matches_preferred(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "archive_list");
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
    assert_eq!(policy.contract_match, "none");
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
fn runtime_contract_snapshot_for_route_uses_route_trace_evidence() {
    let route = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "quantity_comparison".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "README.md | AGENTS.md".to_string(),
            ..IntentOutputContract::default()
        },
    };

    let snapshot = runtime_contract_snapshot_for_route(&route).expect("runtime route snapshot");
    let required = snapshot
        .get("contract")
        .and_then(|value| value.get("required_evidence"))
        .and_then(Value::as_array)
        .expect("required evidence")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();

    assert!(required.contains(&"exists"));
    assert!(required.contains(&"kind"));
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
        if kind.is_registry_capability_bridge() {
            continue;
        }
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
fn route_marker_quantity_comparison_augments_trace_evidence_without_semantic_enum() {
    let route = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "quantity_comparison".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "README.md | AGENTS.md".to_string(),
            ..IntentOutputContract::default()
        },
    };

    let snapshot = trace_snapshot_for_route(&route).expect("route snapshot");
    let required = snapshot
        .get("required_evidence")
        .and_then(Value::as_array)
        .expect("required evidence")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();

    assert!(required.contains(&"exists"));
    assert!(required.contains(&"kind"));
}

#[test]
fn route_effective_contract_marker_prevents_stale_raw_semantic_action_lock() {
    let route = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "contract:workspace_project_summary".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilePaths,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        },
    };

    let policy = action_policy_for_route(
        Some(&route),
        "git_basic",
        &serde_json::json!({"action": "status"}),
    )
    .expect("route policy");

    assert_eq!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::FilePaths
    );
    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        OutputSemanticKind::WorkspaceProjectSummary
    );
    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.contract_match, "workspace_project_summary");
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
fn generic_path_content_allows_git_status_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            response_shape: OutputResponseShape::Strict,
            ..IntentOutputContract::default()
        }),
        "git_basic",
        &serde_json::json!({"action": "status", "path": "/home/guagua/rustclaw"}),
    )
    .expect("action policy");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "git_basic.status");
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
fn generic_delivery_rejects_file_writes() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Filename,
        response_shape: OutputResponseShape::FileToken,
        ..IntentOutputContract::default()
    };

    let policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({
            "action":"write_text",
            "path":"definitely_missing_named_file_golden_001.txt",
            "content":"x"
        }),
    )
    .expect("write policy");

    assert_eq!(policy.decision, ActionPolicyDecision::RejectedNotAllowed);
    assert_eq!(policy.action_key, "fs_basic.write_text");
    assert_eq!(policy.contract_match, "generic_delivery");
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

#[path = "contract_matrix_tests/artifact_and_external_policy.rs"]
mod artifact_and_external_policy;

#[path = "contract_matrix_tests/action_policy_registry.rs"]
mod action_policy_registry;
