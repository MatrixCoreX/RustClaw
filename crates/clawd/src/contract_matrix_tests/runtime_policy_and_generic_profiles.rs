use super::*;

#[test]
fn runtime_contract_snapshot_for_route_uses_route_trace_evidence() {
    let route = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
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
        if kind.is_normalizer_schema_capability_bridge() {
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
        ask_mode: crate::AskMode::act_plain(),
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
fn action_trace_for_route_uses_route_trace_evidence() {
    let route = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
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

    let trace = action_trace_for_route(&route, "fs_basic.stat_paths").expect("route action trace");
    let required = trace
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
fn action_trace_for_archive_capability_ref_supplies_structured_extractor() {
    let route = route_with_machine_capability_ref("capability_ref=archive.list");

    let trace =
        action_trace_for_route(&route, "archive_basic.list").expect("archive route action trace");

    assert_eq!(
        trace.get("contract_match").and_then(Value::as_str),
        Some("capability_ref")
    );
    assert_eq!(
        trace
            .get("observation_extractor")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("archive_basic.list")
    );
    assert_eq!(
        trace
            .get("observation_extractor")
            .and_then(|value| value.get("extractor_kind"))
            .and_then(Value::as_str),
        Some("structured_json")
    );
    let required = trace
        .get("required_evidence")
        .and_then(Value::as_array)
        .expect("required evidence")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(required, vec!["candidates"]);
}

#[test]
fn route_effective_contract_marker_prevents_stale_raw_semantic_action_lock() {
    let route = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
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
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::FilePaths
    );
    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        OutputSemanticKind::WorkspaceProjectSummary
    );
    assert!(policy.is_none());
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
