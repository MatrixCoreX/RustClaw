use super::*;

#[test]
fn unclassified_inline_contract_uses_generic_inline_transform() {
    let output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        ..IntentOutputContract::default()
    };

    let snapshot = runtime_contract_snapshot_for_output_contract(&output_contract)
        .expect("runtime output-contract snapshot");
    assert_eq!(
        snapshot
            .pointer("/contract/contract_match")
            .and_then(Value::as_str),
        Some("generic_inline_transform")
    );
    assert_eq!(
        snapshot
            .pointer("/contract/required_evidence")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        final_answer_shape_for_output_contract(&output_contract),
        Some(FinalAnswerShape::SummaryWithEvidence)
    );
    assert!(compact_prompt_line_for_output_contract(&output_contract)
        .expect("compact prompt line")
        .contains("match=generic_inline_transform"));
    assert_eq!(
        crate::evidence_policy::required_evidence_fields_for_output_contract(&output_contract),
        vec!["field_value"]
    );
}

#[test]
fn unclassified_path_contract_rejects_action_outside_generic_profile() {
    let output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        ..IntentOutputContract::default()
    };

    let trace = action_trace_for_output_contract(&output_contract, "db_basic.schema_version")
        .expect("output-contract action trace");

    assert_eq!(
        trace.get("contract_match").and_then(Value::as_str),
        Some("generic_path_content")
    );
    assert_eq!(
        trace.get("decision").and_then(Value::as_str),
        Some("rejected_not_allowed")
    );
    assert_eq!(
        trace
            .get("observation_extractor")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("db_basic.schema_version")
    );
    assert!(trace
        .get("allowed_actions")
        .and_then(Value::as_array)
        .is_some_and(|actions| actions
            .iter()
            .all(|action| { action.as_str() != Some("db_basic.schema_version") })));
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

    assert!(line.contains("evidence_policy"));
    assert!(line.contains("source=bundled_evidence_policy"));
    assert!(line.contains("planner_authority=agent_loop_registry"));
    assert!(line.contains("match=file_names"));
    assert!(line.contains("required_evidence=candidates"));
    assert!(!line.contains("allowed_actions="));
    assert!(!line.contains("forbidden_actions="));
    assert!(!line.contains("legacy_action_hints="));
    assert!(!line.contains("legacy_forbidden_hints="));
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
        let fallback = fallback_required_evidence_fields_for_output_contract(&output_contract);
        let actual = matrix
            .semantic_contract(*kind)
            .expect("semantic contract")
            .normalized_required_evidence();
        let resolved = required_evidence_for_output_contract(&output_contract)
            .expect("resolved output contract evidence");

        assert_eq!(
            actual,
            resolved,
            "evidence mismatch for `{}`",
            kind.as_str()
        );
        if !fallback.is_empty() {
            assert_eq!(
                actual,
                fallback,
                "fallback mismatch for `{}`",
                kind.as_str()
            );
        }
    }
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

    assert!(err.contains("contract_validation.invalid_field"));
    assert!(err.contains("field=policy_mode"));
}

#[test]
fn contract_runtime_rejects_natural_language_evidence_profile() {
    let source = include_str!("../../../../configs/task_contract_matrix.toml").replace(
        "evidence_profile = \"workspace_user_docs_first\"",
        "evidence_profile = \"read user setup docs first\"",
    );
    let err = parse_contract_matrix_source(&source)
        .expect_err("natural-language evidence profile should fail shape validation");

    assert!(err.contains("contract_validation.invalid_evidence_profile"));
}

#[test]
fn configured_observation_extractors_must_exist_in_registry() {
    let source = format!(
            "{}\n[[contracts.content_excerpt_summary.observation_extractors]]\nsource = \"run_cmd\"\nextractor_kind = \"structured_json\"\n",
            include_str!("../../../../configs/task_contract_matrix.toml")
        );
    let err = parse_contract_matrix_source(&source)
        .expect_err("unregistered explicit extractor should fail validation");

    assert!(err.contains("contract_validation.observation_extractor_registry_missing"));
    assert!(err.contains("source=run_cmd"));
    assert!(err.contains("extractor_kind=structured_json"));
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
