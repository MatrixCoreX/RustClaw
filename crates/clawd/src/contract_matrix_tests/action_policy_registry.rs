use super::*;

#[test]
fn non_bridge_package_actions_remain_structured_contract_inputs() {
    let cases = [(
        OutputSemanticKind::ContentExcerptSummary,
        "package_manager",
        serde_json::json!({"action":"smart_install","packages":["jq"],"dry_run":true}),
        "package_manager.smart_install",
        "content_excerpt_summary",
    )];

    for (semantic_kind, skill, args, expected_action, expected_contract) in cases {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract {
                semantic_kind,
                requires_content_evidence: true,
                response_shape: OutputResponseShape::Strict,
                ..IntentOutputContract::default()
            }),
            skill,
            &args,
        )
        .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.original_action_ref, expected_action);
        assert_eq!(policy.replacement_action_ref, None);
        assert_eq!(policy.contract_repair_source, "none");
        assert_eq!(policy.contract_match, expected_contract);
        assert!(
            policy.required_evidence.iter().all(|token| {
                token
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch.is_ascii_digit())
            }),
            "required evidence should stay machine-tokenized: {:?}",
            policy.required_evidence
        );
    }
}

#[test]
fn generic_inline_transform_allows_transform_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::Strict,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "transform",
        &serde_json::json!({
            "action": "transform_data",
            "data": [{"name":"alpha","score":7},{"name":"beta","score":12}],
            "ops": [{"op":"sort","by":"score","order":"desc"}],
            "output_format": "md_table"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "transform.transform_data");
    assert_eq!(policy.contract_match, "generic_inline_transform");
    assert_eq!(policy.required_evidence, vec!["field_value"]);
}

#[test]
fn generic_inline_transform_allows_kb_namespace_catalog_capability() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::Strict,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "kb",
        &serde_json::json!({"action": "list_namespaces"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "kb.list_namespaces");
    assert_eq!(policy.contract_match, "generic_inline_transform");
    assert_eq!(policy.required_evidence, vec!["field_value"]);
}

#[test]
fn generic_inline_transform_allows_kb_search_capability() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::Strict,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "kb",
        &serde_json::json!({
            "action": "search",
            "namespace": "docs",
            "query": "service status",
            "top_k": 3
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "kb.search");
    assert_eq!(policy.contract_match, "generic_inline_transform");
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
fn content_excerpt_with_summary_allows_supplemental_directory_listing() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptWithSummary,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            .to_string(),
        ..IntentOutputContract::default()
    };

    let list_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"scripts/nl_tests/fixtures/device_local/docs","names_only":true}),
    )
    .expect("list policy");
    assert!(list_policy.is_allowed(), "{list_policy:?}");
    assert_eq!(list_policy.action_key, "fs_basic.list_dir");
    assert_eq!(list_policy.contract_match, "content_excerpt_with_summary");

    let read_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"read_text_range","path":"scripts/nl_tests/fixtures/device_local/docs/release_checklist.md","mode":"head","n":20}),
    )
    .expect("read policy");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert_eq!(read_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(read_policy.contract_match, "content_excerpt_with_summary");
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
fn action_policy_blocks_forbidden_action_for_generic_content_contract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "run_cmd",
        &serde_json::json!({"command":"ls"}),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::RejectedForbidden);
    assert_eq!(policy.contract_match, "generic_path_content");
    assert_eq!(policy.required_evidence, vec!["content_excerpt", "path"]);
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
fn stable_semantic_action_preferences_live_in_task_contract_matrix() {
    let matrix = load_workspace_matrix();
    let cases = [(
        "existence_with_path",
        OutputSemanticKind::ExistenceWithPath,
        "fs_basic.stat_paths",
    )];

    for (contract_name, semantic_kind, preferred_action) in cases {
        let contract = matrix
            .semantic_contract(semantic_kind)
            .unwrap_or_else(|| panic!("missing contract for {contract_name}"));
        assert!(
            contract
                .preferred_actions
                .iter()
                .any(|action| action == preferred_action),
            "contract `{contract_name}` should prefer `{preferred_action}`, got {:?}",
            contract.preferred_actions
        );
        assert!(
            contract
                .allowed_actions
                .iter()
                .any(|action| action == preferred_action),
            "contract `{contract_name}` should allow `{preferred_action}`, got {:?}",
            contract.allowed_actions
        );
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
fn legacy_canonicalization_records_original_and_replacement_refs() {
    let route = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..IntentOutputContract::default()
    };
    let policy =
        action_policy_for_output_contract(Some(&route), "read_file", &json!({"path":"README.md"}))
            .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.original_action_ref, "read_file");
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
    assert_eq!(
        policy.replacement_action_ref.as_deref(),
        Some("fs_basic.read_text_range")
    );
    assert_eq!(
        policy.contract_repair_source,
        "legacy_tool_canonicalization"
    );
    assert_eq!(
        policy.preferred_replacement_reason_code.as_deref(),
        Some("legacy_tool_canonical_action_allowed")
    );
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
fn matrix_generated_cases_cover_current_unique_contract_paths() {
    let matrix = load_workspace_matrix();
    let cases = generated_contract_cases(&matrix, 33);

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
