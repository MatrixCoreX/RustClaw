use super::*;

#[test]
fn active_bound_inventory_path_overrides_bare_path_directory_listing_contract() {
    let state = test_state();
    let task = claimed_task("task-active-bound-inventory-path");
    let mut route = free_route_result();
    route.resolved_intent = "List contents of directory scripts/nl_tests/fixtures/locator_smart/case_only\n\n### ACTIVE_EXECUTION_ANCHOR\nfollowup_source_request: find report\nfollowup_op_kind: Read\nfollowup_bound_target: case_only/report.md\nobserved_bound_target: case_only/report.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/locator_smart/case_only".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":1,"total":1},"entries":[{"kind":"file","name":"Report.MD","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","size_bytes":33}],"names":["Report.MD"],"names_by_kind":{"dirs":[],"files":["Report.MD"],"other":[]},"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only","resolved_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only"}"#,
    ));

    let (answer, _) = direct_path_from_active_bound_inventory(&loop_state, Some(&ctx))
        .expect("active bound target should select matching inventory entry path");
    assert_eq!(
        answer,
        "scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD"
    );

    let mut delivery = vec!["Report.MD".to_string()];
    let mut finalizer_summary = None;
    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "scripts/nl_tests/fixtures/locator_smart/case_only",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );
    assert_eq!(
        delivery,
        vec!["scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD"]
    );
}

#[test]
fn matrix_shape_guard_replaces_unstructured_strict_list_with_observed_list() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-list");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":2,"ext":"md","results":["alpha.md","beta.md"],"root":"document"}"#,
    ));
    let mut delivery = vec!["document 目录下有 alpha.md 和 beta.md。".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "列出 document 下的 md 文件名，只输出列表",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("alpha.md\nbeta.md")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_strict_list_shape_builds_list_from_observed_json() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":2,"ext":"md","results":["document/beta.md","document/alpha.md"],"root":"document"}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("matrix list answer");

    assert_eq!(answer, "alpha.md\nbeta.md");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_archive_member_list_filters_file_entries_from_structured_kinds() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/test_bundle.zip".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveList;
    route
        .output_contract
        .self_extension
        .list_selector
        .target_kind = crate::OutputScalarCountTargetKind::File;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        r#"{"extra":{"action":"list","archive":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","entries":[{"kind":"file","name":"notes.txt"},{"kind":"file","name":"nested/config.ini"},{"kind":"dir","name":"home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/manual_dynamic_guard_unpack/"},{"kind":"file","name":"home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/manual_dynamic_guard_unpack/notes.txt"}],"candidates":["notes.txt","nested/config.ini","home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/manual_dynamic_guard_unpack/"]},"text":"ignored"}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("archive member list answer");

    assert_eq!(
        answer,
        "manual_dynamic_guard_unpack/notes.txt\nnested/config.ini\nnotes.txt"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_archive_member_list_defaults_untyped_selector_to_file_entries() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/test_bundle.zip".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveList;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        r#"{"action":"list","archive":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","entries":[{"kind":"file","name":"notes.txt"},{"kind":"dir","name":"manual_dynamic_guard_unpack/"},{"kind":"file","name":"nested/config.ini"}],"candidates":["notes.txt","manual_dynamic_guard_unpack/","nested/config.ini"]}"#,
    ));

    let (answer, _summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("archive member list answer");

    assert_eq!(answer, "nested/config.ini\nnotes.txt");
}

#[test]
fn matrix_archive_member_list_replaces_synthesis_with_observed_projection() {
    let state = test_state();
    let task = claimed_task("task-archive-member-list-observed-projection");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/test_bundle.zip".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveList;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        r#"{"action":"list","archive":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","entries":[{"kind":"file","name":"notes.txt"},{"kind":"dir","name":"manual_dynamic_guard_unpack/"},{"kind":"file","name":"nested/config.ini"}],"candidates":["notes.txt","manual_dynamic_guard_unpack/","nested/config.ini"]}"#,
    ));
    let mut delivery =
        vec!["notes.txt\nnested/config.ini\nmanual_dynamic_guard_unpack/".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "list archive members",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["nested/config.ini\nnotes.txt"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("nested/config.ini\nnotes.txt")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_strict_list_shape_builds_hidden_entry_list_from_inventory() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":2,"hidden":2,"total":3},"entries":[{"hidden":true,"kind":"dir","name":".git","path":".git"},{"hidden":true,"kind":"file","name":".gitignore","path":".gitignore"},{"hidden":false,"kind":"file","name":"README.md","path":"README.md"}],"include_hidden":true,"names":[".git",".gitignore","README.md"],"path":"."}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("matrix hidden entries answer");

    assert_eq!(answer, ".git\n.gitignore");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_grouped_name_list_shape_builds_groups_from_names_by_kind() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "workspace".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":5,"files":2,"total":7},"names_by_kind":{"files":["README.md","package.json"],"dirs":["configs","data","docs","logs","tmp"],"other":[]},"path":"workspace"}"#,
    ));

    let (answer, summary) =
        super::super::matrix_grouped_name_list_observed_answer(&route, &loop_state)
            .expect("matrix grouped name answer");

    assert_eq!(
        answer,
        "dirs:\n- configs\n- data\n- docs\n- logs\n- tmp\nfiles:\n- package.json\n- README.md"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_grouped_name_list_shape_reads_wrapped_inventory_extra() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "workspace".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":2,"files":3,"total":5},"names_by_kind":{"files":["README.md","Cargo.lock","Cargo.toml"],"dirs":["configs","crates"],"other":[]},"path":"workspace"},"text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":2,\"files\":3,\"total\":5},\"names_by_kind\":{\"files\":[\"README.md\",\"Cargo.lock\",\"Cargo.toml\"],\"dirs\":[\"configs\",\"crates\"],\"other\":[]},\"path\":\"workspace\"}"}"#,
    ));

    assert!(super::super::directory_entry_groups_prefers_observed_groups(&route, &loop_state));
    let (answer, summary) =
        super::super::matrix_grouped_name_list_observed_answer(&route, &loop_state)
            .expect("matrix grouped name answer");

    assert!(answer.contains("Cargo.toml"));
    assert_eq!(
        answer,
        "dirs:\n- configs\n- crates\nfiles:\n- Cargo.lock\n- Cargo.toml\n- README.md"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn mixed_listing_contract_prefers_grounded_synthesis_after_file_read() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let answer = "这个仓库的 UI 更像一个独立前端，因为 UI/package.json 的 name 是 react-example，并且 UI 目录有独立构建脚本。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"names_by_kind":{"files":["Cargo.toml"],"dirs":["UI"],"other":[]},"path":"."}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"UI/package.json","excerpt":"1|{\n2|  \"name\": \"react-example\"\n3|}"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", answer));
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());

    let (actual, summary) =
        super::super::latest_grounded_synthesis_for_mixed_listing_contract(&route, &loop_state)
            .expect("mixed evidence synthesis");

    assert_eq!(actual, answer);
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn matrix_shape_guard_replaces_unstructured_grouped_name_list_with_observed_groups() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-grouped-name-list");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "workspace".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"files":["README.md"],"dirs":["configs","docs"],"other":[]},"path":"workspace"}"#,
    ));
    let mut delivery = vec!["workspace 下面有 configs、docs 和 README.md。".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "list direct children grouped by kind",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(
        delivery,
        vec!["dirs:\n- configs\n- docs\nfiles:\n- README.md"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("dirs:\n- configs\n- docs\nfiles:\n- README.md")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_shape_replacement_only_triggers_for_bad_finalizer_summary() {
    let good_summary = crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        ..Default::default()
    };
    let bad_summary = crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        contract_ok: false,
        completion_ok: Some(false),
        grounded_ok: Some(false),
        format_ok: Some(false),
        needs_clarify: Some(true),
        ..Default::default()
    };

    assert!(
        !super::super::finalizer_summary_requires_matrix_observed_replacement(Some(&good_summary))
    );
    assert!(
        super::super::finalizer_summary_requires_matrix_observed_replacement(Some(&bad_summary))
    );
    assert!(!super::super::finalizer_summary_requires_matrix_observed_replacement(None));
}

#[test]
fn matrix_shape_guard_replaces_unstructured_table_with_markdown_table() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-table");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "data/app.sqlite".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let mut delivery = vec!["数据库里有 orders 和 users 两张表。".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "列出数据库表，输出表格",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["| name |\n| --- |\n| orders |\n| users |"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("| name |\n| --- |\n| orders |\n| users |")
    );
    assert!(finalizer_summary.is_some());
}
