#[test]
fn structured_observed_body_includes_path_batch_metadata_for_synthesis() {
    let body = r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","modified_ts":1777345844,"path":"Cargo.lock","resolved_path":"/tmp/repo/Cargo.lock","size_bytes":121657},"path":"/tmp/repo/Cargo.lock"},{"exists":true,"fact":{"kind":"file","modified_ts":1777357772,"path":"Cargo.toml","resolved_path":"/tmp/repo/Cargo.toml","size_bytes":2606},"path":"/tmp/repo/Cargo.toml"}],"include_missing":true}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "path_batch_facts\npath_fact name=Cargo.lock path=/tmp/repo/Cargo.lock exists=true kind=file size_bytes=121657 modified_ts=1777345844\npath_fact name=Cargo.toml path=/tmp/repo/Cargo.toml exists=true kind=file size_bytes=2606 modified_ts=1777357772"
            )
        );
}

#[test]
fn structured_observed_body_includes_inventory_dir_entry_metadata_for_synthesis() {
    let body = r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","sort_by":"size_desc"}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "inventory_dir path=/tmp/repo/prompts/schemas sort_by=size_desc total=2 files=2 dirs=0 hidden=0\nentry name=intent_normalizer.schema.json kind=file size_bytes=9402 modified_ts=1777513843\nentry name=plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
            )
        );
}

#[test]
fn structured_observed_body_includes_inventory_dir_size_summary_for_synthesis() {
    let body = r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","size_summary":{"largest_file":{"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},"matched_file_count":2,"smallest_file":{"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187},"total_file_size_bytes":13589},"sort_by":"name"}"#;
    let observed = structured_observed_body("system_basic", body).expect("observed body");
    assert!(observed.contains("size_summary.matched_file_count=2"));
    assert!(observed.contains("size_summary.total_file_size_bytes=13589"));
    assert!(observed.contains(
        "size_summary.largest_file name=intent_normalizer.schema.json path=prompts/schemas/intent_normalizer.schema.json kind=file size_bytes=9402 modified_ts=1777513843"
    ));
    assert!(observed.contains(
        "size_summary.smallest_file name=plan_result.schema.json path=prompts/schemas/plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
    ));
}

#[test]
fn structured_observed_body_unwraps_extra_inventory_dir_for_synthesis() {
    let body = r#"{"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","size_summary":{"largest_file":{"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},"matched_file_count":2,"total_file_size_bytes":13589},"sort_by":"name"},"text":"raw wrapper fallback text"}"#;
    let observed = structured_observed_body("fs_basic", body).expect("observed body");
    assert!(observed.starts_with(
        "inventory_dir path=/tmp/repo/prompts/schemas sort_by=name total=2 files=2 dirs=0 hidden=0"
    ));
    assert!(observed.contains("size_summary.largest_file name=intent_normalizer.schema.json"));
    assert!(observed.contains(
        "entry name=plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
    ));
    assert!(!observed.contains("raw wrapper fallback text"));
}

#[test]
fn structured_observed_body_includes_inventory_dir_names_by_kind_for_synthesis() {
    let body = r#"{"extra":{"action":"inventory_dir","counts":{"dirs":4,"files":1,"hidden":0,"total":5},"names_by_kind":{"dirs":["agent_rollout_metrics","base_skill_contracts_20260516_100540","base_skill_contracts_20260516_112927","base_skill_contracts_20260527_042323"],"files":["act_plan.log"],"other":[]},"path":"logs","resolved_path":"/tmp/repo/logs","sort_by":"name"},"text":"{}"}"#;
    let observed = structured_observed_body("fs_basic", body).expect("observed body");
    assert!(observed.starts_with(
        "inventory_dir path=/tmp/repo/logs sort_by=name total=5 files=1 dirs=4 hidden=0"
    ));
    assert!(observed.contains(
        "dir_entries=agent_rollout_metrics,base_skill_contracts_20260516_100540"
    ));
    assert!(observed.contains("file_entries=act_plan.log"));
    assert!(!observed.contains("raw wrapper fallback text"));
}

#[test]
fn structured_observed_body_compacts_large_inventory_dir_by_kind() {
    let entries = (0..9)
        .map(|idx| {
            serde_json::json!({
                "hidden": false,
                "kind": "dir",
                "modified_ts": 1777513843,
                "name": format!("dir_{idx}"),
                "path": format!("dir_{idx}"),
                "size_bytes": 0
            })
        })
        .chain((0..9).map(|idx| {
            serde_json::json!({
                "hidden": false,
                "kind": "file",
                "modified_ts": 1777513843,
                "name": format!("file_{idx}.md"),
                "path": format!("file_{idx}.md"),
                "size_bytes": 42
            })
        }))
        .collect::<Vec<_>>();
    let body = serde_json::json!({
        "action": "inventory_dir",
        "counts": {"dirs": 9, "files": 9, "hidden": 0, "total": 18},
        "entries": entries,
        "path": ".",
        "resolved_path": "/tmp/repo",
        "sort_by": "name"
    })
    .to_string();

    let observed = structured_observed_body("system_basic", &body).expect("observed body");
    assert!(observed.contains("dir_entries=dir_0:size_bytes=0,dir_1:size_bytes=0"));
    assert!(observed.contains("file_entries=file_0.md:size_bytes=42,file_1.md:size_bytes=42"));
    assert!(!observed.contains("modified_ts=1777513843"));
    assert!(observed.contains("size_bytes=42"));
}

#[test]
fn structured_observed_body_includes_count_inventory_breakdown_for_synthesis() {
    let body = r#"{"action":"count_inventory","counts":{"dirs":26,"files":40,"hidden":0,"total":66},"kind_filter":"any","path":".","resolved_path":"/tmp/repo"}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "action=count_inventory\npath=.\nresolved_path=/tmp/repo\nkind_filter=any\ncount_files=40\ncount_dirs=26\ncount_total=66\ncount_hidden=0"
            )
        );
}

#[test]
fn direct_scalar_defers_route_locator_hint_quantity_comparison_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "list_dir", "a\nb\nc\n"));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "scripts".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_scalar_defers_compare_paths_result_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"compare_paths","left":{"path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","kind":"file","size_bytes":123},"right":{"path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","kind":"file","size_bytes":456},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":-333,"left_newer":null,"same_content":false}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "Cargo.lock|Cargo.toml".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
    assert!(
        has_observed_answer_candidates(&loop_state),
        "compare_paths should remain available as observed facts for synthesis"
    );
}

#[test]
fn quantity_comparison_does_not_force_direct_scalar_observed_answer() {
    let route = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "Cargo.lock|Cargo.toml".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    assert!(!super::route_prefers_direct_observed_answer_for_scalar(
        &route
    ));
}

#[test]
fn direct_answer_defers_git_status_dirty_worktree_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n",
    ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_log_release_note_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        "RustClaw is a local Rust agent runtime centered on clawd.",
    ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","field_path":"workspace.package.version","value_text":"0.1.7"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_3",
            "git_basic",
            "exit=0\n09342a6a fix: expose nl execution and locator flows\n336e8d92 docs: update planner-first architecture diagrams\n",
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "RustClaw".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_status_clean_when_exit_only_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "git_basic", "exit=0\n"));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_status_dirty_without_branch_header_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        " M Cargo.toml\n?? new_file.txt\n",
    ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}
