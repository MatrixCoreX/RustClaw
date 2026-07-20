use super::{
    collect_machine_text_fragments_from_output,
    collect_requested_machine_kv_surfaces_from_state_patch, exact_machine_field_selector,
    parse_machine_kv_units, requested_machine_kv_summary_from_observations,
    structured_json_satisfies_field_selector,
};

#[test]
fn machine_summary_projects_policy_decision_fields() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"confirmation_required":false,"decision":"deny","reason_codes":["sudo_not_allowed"],"risk_level":"high","would_execute":false}"#,
        &mut observed,
    );

    assert_eq!(
        requested_machine_kv_summary_from_observations(
            "decision risk_level confirmation_required reason_codes",
            &observed,
        )
        .as_deref(),
        Some(
            r#"decision=deny risk_level=high confirmation_required=false reason_codes=["sudo_not_allowed"]"#
        )
    );
}

#[test]
fn machine_summary_projects_requested_config_fields_without_echoing_target_value() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"llm.selected_vendor","resolved_field_path":"llm.selected_vendor","value":"minimax","value_text":"minimax"}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "configs/config.toml에서 llm.selected_vendor 값을 읽고 field_path와 value만 반환하세요.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("field_path=llm.selected_vendor value=minimax")
    );
}

#[test]
fn machine_summary_projects_config_preview_before_and_after_fields() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"after":"minimax","applied":false,"before":"minimax","dry_run":true,"field_path":"llm.selected_vendor","path":"configs/config.toml","would_change":false}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "dry_run field_path before after",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("dry_run=true field_path=llm.selected_vendor before=minimax after=minimax")
    );
}

#[test]
fn machine_summary_projects_structured_directory_listing_aliases() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"names_by_kind":{"dirs":[],"files":["release_checklist.md","service_notes.md"],"other":[]}}}"#,
        &mut observed,
    );

    let summary =
        requested_machine_kv_summary_from_observations("Return names and count only.", &observed);

    assert_eq!(
        summary.as_deref(),
        Some(r#"names=["release_checklist.md","service_notes.md"] count=2"#)
    );
}

#[test]
fn machine_summary_accepts_grounded_command_with_path_continuation() {
    let observed =
        vec!["144|Use the auto-sync script: `python3 scripts/sync_skill_docs.py`.".to_string()];

    let summary = requested_machine_kv_summary_from_observations(
        "Answer exactly as machine summary: command=python3 scripts/sync_skill_docs.py.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("command=python3 scripts/sync_skill_docs.py")
    );
}

#[test]
fn machine_summary_accepts_inline_machine_enums_and_observed_script() {
    let observed =
        vec!["212|When changing Rust code, run `python3 scripts/check_long_files.py`.".to_string()];

    let summary = requested_machine_kv_summary_from_observations(
        "Answer exactly: hard_ceiling_lines=2000 script=check_long_files.py.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("hard_ceiling_lines=2000 script=check_long_files.py")
    );
}

#[test]
fn machine_summary_accepts_comma_list_machine_literals() {
    let observed = vec![
        "Prefer registry_metadata,INTERFACE.md,generated_prompts over clawd_main_flow.".to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Only answer: prefer=registry_metadata,INTERFACE.md,generated_prompts over=clawd_main_flow.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("prefer=registry_metadata,INTERFACE.md,generated_prompts over=clawd_main_flow")
    );
}

#[test]
fn machine_summary_accepts_nested_machine_token_value() {
    let observed = vec![
        "88|- `kind=run_skill` does not run the intent normalizer or planner / agent loop."
            .to_string(),
        "95|| Does it enter the planner / agent loop? | Yes. | No.".to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Only answer: run_skill=kind=run_skill planner=No.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("run_skill=kind=run_skill planner=No")
    );
}

#[test]
fn machine_summary_does_not_require_pair_value_as_standalone_marker() {
    let observed = vec![
        "22|| `semantic_rewrite` | Migration debt only | Ordinary intent/action/skill decision outside planner. |"
            .to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Answer exactly as machine summary: owner=semantic_rewrite status=Migration.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("owner=semantic_rewrite status=Migration")
    );
}

#[test]
fn machine_summary_projects_pair_values_from_read_range_json_excerpt() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"read_range","end_line":22,"excerpt":"22|| `semantic_rewrite` | Migration debt only | Ordinary intent/action/skill decision outside planner. |","mode":"range","path":"/repo/docs/runtime_semantic_rewrite_inventory.md","start_line":22}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Use read_range only for /repo/docs/runtime_semantic_rewrite_inventory.md line 22. Answer exactly as machine summary: owner=semantic_rewrite status=Migration.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("owner=semantic_rewrite status=Migration")
    );
}

#[test]
fn machine_summary_projects_status_from_status_code_alias() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"status_code":200,"output_path":"/tmp/example.body"}}"#,
        &mut observed,
    );

    let summary =
        requested_machine_kv_summary_from_observations("Return status and output_path.", &observed);

    assert_eq!(
        summary.as_deref(),
        Some("status=200 output_path=/tmp/example.body")
    );
}

#[test]
fn machine_summary_projects_cli_placeholder_for_required_machine_field() {
    let observed = vec![
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue"
            .to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Return required machine field resume_task_id.",
        &observed,
    );

    assert_eq!(summary.as_deref(), Some("resume_task_id=<TASK_ID>"));
}

#[test]
fn machine_summary_prefers_cli_placeholder_over_none_machine_value() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        "clawcli resume is available.\n\nresume_task_id=<none>",
        &mut observed,
    );
    collect_machine_text_fragments_from_output(
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Return required machine field resume_task_id.",
        &observed,
    );

    assert_eq!(summary.as_deref(), Some("resume_task_id=<TASK_ID>"));
}

#[test]
fn machine_summary_does_not_project_placeholder_without_matching_field_suffix() {
    let observed = vec!["Usage: demo --text <TEXT> <TASK_ID>".to_string()];

    let summary = requested_machine_kv_summary_from_observations(
        "Return required machine field checkpoint_id.",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_preserves_dotted_markers_and_embedded_pairs() {
    let observed = vec![
        "task_control.resume.dry_run task_control.pause.dry_run checkpoint_id=ckpt-1 task_id=00000000-0000-4000-8000-000000000010 pause_seconds=120 would_mutate=false"
            .to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Preview task_control.resume(checkpoint_id=ckpt-1) and task_control.pause(pause_seconds=120). Final must contain task_control.resume.dry_run task_control.pause.dry_run and checkpoint_id. task_id=00000000-0000-4000-8000-000000000010",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("task_control.resume.dry_run task_control.pause.dry_run task_id=00000000-0000-4000-8000-000000000010 checkpoint_id=ckpt-1 pause_seconds=120")
    );
}

#[test]
fn machine_summary_does_not_accept_marker_substring() {
    let observed = vec![
        "task_control.resume.dry_run_extra checkpoint_id=ckpt-1 task_id=00000000-0000-4000-8000-000000000010"
            .to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Final must contain task_control.resume.dry_run and checkpoint_id=ckpt-1.",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_preserves_requested_underscore_field_markers_from_json_keys() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"adapter_kind":"local_process_poll","cancel_ref":"optional_cancel_reference","status":"cancelled","terminal_projection":{"state":"cancelled","terminal":true}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "必须包含 cancel_ref、adapter_kind=local_process_poll、status=cancelled 和 terminal_projection。",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(
            r#"cancel_ref=optional_cancel_reference terminal_projection={"state":"cancelled","terminal":true} adapter_kind=local_process_poll status=cancelled"#
        )
    );
}

#[test]
fn machine_summary_projects_multiline_weather_machine_fields() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        "location=北京\ntemperature=25.4\nweather_code=多云",
        &mut observed,
    );
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"location":"北京","temperature":25.4,"weather_code":"多云","weather_code_raw":3}}"#,
        &mut observed,
    );
    observed.sort();
    observed.dedup();

    let summary = requested_machine_kv_summary_from_observations(
        "只返回 location、temperature、weather_code。",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("location=北京 temperature=25.4 weather_code=多云")
    );
}

#[test]
fn machine_text_fragments_do_not_parse_json_hidden_in_visible_text() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"text":"{\"command_output\":\"hidden\",\"status\":\"ok\"}"}"#,
        &mut observed,
    );

    assert!(
        !observed.iter().any(|value| matches!(
            value.as_str(),
            "command_output" | "command_output=hidden" | "status=ok"
        )),
        "{observed:?}"
    );
}

#[test]
fn machine_text_fragments_accept_extra_machine_fields() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"command_output":"visible","status":"ok"},"text":"display only"}"#,
        &mut observed,
    );

    assert!(observed
        .iter()
        .any(|value| value == "command_output=visible"));
    assert!(observed.iter().any(|value| value == "extra.status=ok"));
}

#[test]
fn machine_summary_projects_bulleted_candidate_count_field() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        "照片整理来源候选发现与预览完成。\n\n- candidate_count=7\n- mode=plan\n- path=/home/guagua/rustclaw/plan",
        &mut observed,
    );
    observed.sort();
    observed.dedup();

    let summary = requested_machine_kv_summary_from_observations(
        "返回 candidate_count、mode=plan。",
        &observed,
    );

    assert_eq!(summary.as_deref(), Some("candidate_count=7 mode=plan"));
}

#[test]
fn machine_summary_does_not_duplicate_following_inline_machine_pair() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"planned_groups=[{"group":"generated_images","files":["document/gen-1.png"]}], would_move=false"#,
        &mut observed,
    );
    collect_machine_text_fragments_from_output(
        r#"{"planned_groups":[{"group":"generated_images","files":["document/gen-1.png"]}],"would_move":false}"#,
        &mut observed,
    );
    observed.sort();
    observed.dedup();

    let summary = requested_machine_kv_summary_from_observations(
        "return planned_groups and would_move=false",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(
            r#"planned_groups=[{"files":["document/gen-1.png"],"group":"generated_images"}] would_move=false"#
        )
    );
}

#[test]
fn machine_summary_ignores_filename_like_dotted_markers() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"path":"tmp/nl_basic_skill_100_write_case/note.txt","target":"note.txt","result":"ok"}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "创建 note.txt，写入 alpha，再追加 beta，最后删除目录；用机器字段汇总每步状态。",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_ignores_sqlite_filename_markers() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"sqlite_query","db_path":"/tmp/test_contract.sqlite","result":{"columns":["id"],"rows":[{"id":1}]}},"text":"test_contract.sqlite"}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "query test_contract.sqlite and summarize rows",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_projects_requested_single_field_markers_from_json_scalars() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"path":"/home/guagua/rustclaw/README.md","exists":true,"size_bytes":12345}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "只返回 path、exists、size_bytes 三个机器字段。",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("path=/home/guagua/rustclaw/README.md exists=true size_bytes=12345")
    );
}

#[test]
fn machine_summary_projects_sqlite_table_count_and_tables() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"list_tables","table_count":3,"tables":["orders","service_logs","users"],"field_value":{"table_count":3,"tables":["orders","service_logs","users"]}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "列出表名，返回 table_count 和 tables。",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(r#"table_count=3 tables=["orders","service_logs","users"]"#)
    );
}

#[test]
fn machine_summary_projects_requested_value_template_from_structured_scalar() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"extract_field","field_path":"name","value":"rustclaw-nl-fixture","value_text":"rustclaw-nl-fixture","value_type":"string"}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Extract the name field and answer package_name=<value>.",
        &observed,
    );

    assert_eq!(summary.as_deref(), Some("package_name=rustclaw-nl-fixture"));
}

#[test]
fn machine_summary_rejects_requested_value_template_when_scalar_is_ambiguous() {
    let observed = vec![
        "value=alpha".to_string(),
        "value=beta".to_string(),
        "field_path=name".to_string(),
    ];

    let summary =
        requested_machine_kv_summary_from_observations("Only answer result=<value>.", &observed);

    assert!(summary.is_none());
}

#[test]
fn machine_summary_projects_read_range_path_and_total_lines_markers() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"read_range","path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md","resolved_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md","start_line":1,"end_line":7,"total_lines":7,"excerpt":"1|# Service Notes\n2|."}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "最终只回答机器字段 path 和 total_lines。",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("path=/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md total_lines=7")
    );
}

#[test]
fn machine_summary_projects_grep_match_line_markers() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"action":"grep_text","match_count":1,"matches":[{"path":"docs/service_notes.md","line":5,"text":"status=ready"}]}"#,
        &mut observed,
    );
    observed.sort();
    observed.dedup();

    let summary = requested_machine_kv_summary_from_observations(
        "Return path, line, line_number.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("path=docs/service_notes.md line=5 line_number=5")
    );
}

#[test]
fn machine_summary_requires_exact_line_pair_evidence() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"action":"grep_text","match_count":1,"matches":[{"path":"docs/service5_notes.md","line":7,"text":"status=ready"}]}"#,
        &mut observed,
    );
    observed.sort();
    observed.dedup();

    let summary = requested_machine_kv_summary_from_observations(
        "Return exact machine pair line=5.",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_requires_value_projection_for_single_field_marker() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"count_inventory","counts":{"total":2}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations("只输出 count 字段。", &observed);

    assert!(summary.is_none());
}

#[test]
fn machine_summary_ignores_request_option_pairs_when_listing_results() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"names":["release_checklist.md","service_notes.md"],"names_only":true,"max_entries":10}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "List only the filenames, names_only=true, max_entries=10, and do not summarize prose.",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_projects_requested_array_marker_from_json_values() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"names":["release_checklist.md","service_notes.md"]}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "List only the filenames under docs and return names.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(r#"names=["release_checklist.md","service_notes.md"]"#)
    );
}

#[test]
fn machine_unit_parser_preserves_balanced_json_container_values() {
    assert_eq!(
        parse_machine_kv_units(r#"count=4 names=["alpha","beta"] metadata={"source":"observed"}"#),
        vec![
            "count=4",
            r#"names=["alpha","beta"]"#,
            r#"metadata={"source":"observed"}"#
        ]
    );
    assert_eq!(
        parse_machine_kv_units("(status=ok) [count=4]"),
        vec!["status=ok", "count=4"]
    );
}

#[test]
fn machine_summary_projects_task_control_lifecycle_markers() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"count":0,"states":"none","can_poll":false,"can_cancel":false,"checkpoint_id_present":false,"field_value":{"count":0,"states":"none","can_poll":false,"can_cancel":false,"checkpoint_id_present":false}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "只输出 count、states、can_poll 三个字段。",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("count=0 states=none can_poll=false")
    );
}

#[test]
fn machine_summary_preserves_requested_nested_machine_contract() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"resume_entrypoint":"checkpoint_declared","lease":{"required":true,"scope":"resume_execution","mode":"renewable","seconds_source":"runtime_config","heartbeat_renewal":true}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Return resume_entrypoint and lease fields.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(
            r#"resume_entrypoint=checkpoint_declared lease={"heartbeat_renewal":true,"mode":"renewable","required":true,"scope":"resume_execution","seconds_source":"runtime_config"}"#
        )
    );
}

#[test]
fn machine_summary_preserves_complete_coding_repair_contract() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"field_value":{"checkpoint":{"checkpoint_ref":"dry_run:checkpoint:pre_patch"},"diff":{"diff_ref":"dry_run:diff:repair_patch"},"failed_verification":{"status":"failed"},"repair_attempt":{"attempt":1},"passing_verification":{"status":"passed"},"rewind_references":["dry_run:checkpoint:pre_patch","dry_run:diff:repair_patch"]}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Return checkpoint, diff, failed_verification, repair_attempt, passing_verification, and rewind_references.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(
            r#"checkpoint={"checkpoint_ref":"dry_run:checkpoint:pre_patch"} diff={"diff_ref":"dry_run:diff:repair_patch"} failed_verification={"status":"failed"} repair_attempt={"attempt":1} passing_verification={"status":"passed"} rewind_references=["dry_run:checkpoint:pre_patch","dry_run:diff:repair_patch"]"#
        )
    );
}

#[test]
fn machine_summary_projects_requested_media_duration() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"provider":"fixture","duration":10,"resolution":"720P"}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Return provider, duration, and resolution.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("provider=fixture duration=10 resolution=720P")
    );
}

#[test]
fn machine_summary_rejects_nested_contract_with_visible_text_boundary() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"lease":{"mode":"renewable","text":"provider response"}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations("Return lease.", &observed);

    assert_eq!(summary, None);
}

#[test]
fn machine_summary_requires_values_for_task_control_presence_markers() {
    let observed = vec![
        "count=0".to_string(),
        "can_poll".to_string(),
        "checkpoint_id_present".to_string(),
    ];

    let summary = requested_machine_kv_summary_from_observations(
        "Return count, can_poll, checkpoint_id_present.",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn state_patch_machine_summary_surfaces_ignore_runtime_status_query_kind() {
    let mut surfaces = Vec::new();
    collect_requested_machine_kv_surfaces_from_state_patch(
        &serde_json::json!({
            "runtime_status_query": {"kind": "awaiting_user_approval", "scope": "session"},
            "required_machine_fields": ["can_poll", "can_cancel"],
            "required_field": "state=completed"
        }),
        &mut surfaces,
    );

    assert!(surfaces.contains(&"can_cancel can_poll".to_string()));
    assert!(surfaces.contains(&"state=completed".to_string()));
    assert!(!surfaces
        .iter()
        .any(|surface| surface.contains("awaiting_user_approval")));
}

#[test]
fn machine_summary_projects_archive_member_list_markers() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"list","member_count":2,"members":["notes.txt","nested/config.ini"],"field_value":{"member_count":2,"members":["notes.txt","nested/config.ini"]}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "只返回 member_count 和 members。",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(r#"member_count=2 members=["notes.txt","nested/config.ini"]"#)
    );
}

#[test]
fn machine_summary_projects_archive_member_read_markers() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"read","path":"notes.txt","member_path":"notes.txt","content_excerpt":"fixture_archive_notes","field_value":{"member_path":"notes.txt","content_excerpt":"fixture_archive_notes"}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Return member_path and content_excerpt.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some("member_path=notes.txt content_excerpt=fixture_archive_notes")
    );
}

#[test]
fn machine_summary_projects_archive_content_excerpt_with_spaces() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"read","member":"notes.txt","path":"notes.txt","content_excerpt":"fixture archive notes","field_value":{"member":"notes.txt","path":"notes.txt","content_excerpt":"fixture archive notes"}}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "Return member path and content_excerpt.",
        &observed,
    );

    assert_eq!(
        summary.as_deref(),
        Some(r#"member=notes.txt path=notes.txt content_excerpt="fixture archive notes""#)
    );
}

#[test]
fn machine_summary_ignores_action_name_markers_when_content_result_is_required() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"read_range","path":"scripts/nl_tests/fixtures/device_local/docs/service_notes.md","line_count":6}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "用 read_range 读取 scripts/nl_tests/fixtures/device_local/docs/service_notes.md 第 1 到 6 行，回答必须包含文件路径和读取到的行数。",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_ignores_skill_identity_marker_without_output_field() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"skill":"install_module","action":"install","module":"requests","dry_run":true,"commands":["python3 -m pip install --user requests"]},"text":"skill=install_module\naction=install\nmodule=requests\ndry_run=true"}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "只给出 install_module 的 dry-run 计划和生态判断，不要实际安装。",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_ignores_action_identity_marker_without_output_field() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"action":"assess_gap","recommended_mode":"manual_review","safe_defaults":{"does_not_enable_new_skill":true,"does_not_modify_runtime":true}},"text":"Need an explicit extension mode before making changes."}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "只做 assess_gap，不创建文件、不注册。",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_ignores_read_range_slice_option_pairs() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"path":"scripts/nl_tests/fixtures/device_local/docs/service_notes.md","line_count":6,"slice_mode":"range","slice_start":1,"slice_end":6}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations(
        "slice_mode=range slice_start=1 slice_end=6",
        &observed,
    );

    assert!(summary.is_none());
}

#[test]
fn machine_summary_ignores_read_range_slice_n_option_pair() {
    let mut observed = Vec::new();
    collect_machine_text_fragments_from_output(
        r#"{"extra":{"path":"scripts/nl_tests/fixtures/device_local/docs/archive/README.txt","requested_n":5,"total_lines":2,"excerpt":"1|Archive fixtures for NL tests."}}"#,
        &mut observed,
    );

    let summary = requested_machine_kv_summary_from_observations("slice_n=5", &observed);

    assert!(summary.is_none());
}

#[test]
fn single_inline_flag_pair_still_requires_observed_value() {
    let observed = vec!["This line does not contain the requested flag.".to_string()];

    let summary =
        requested_machine_kv_summary_from_observations("Only answer: required=yes.", &observed);

    assert!(summary.is_none());
}

#[test]
fn structured_selector_requires_every_field_in_machine_payload() {
    let output = r#"{"extra":{"checkpoint":{"status":"planned"},"diff":{"status":"planned"},"failed_verification":{"status":"failed"},"repair_attempt":{"attempt":1},"passing_verification":{"status":"passed"},"rewind_references":["checkpoint:1"]},"text":"localized fallback"}"#;

    assert!(structured_json_satisfies_field_selector(
        "checkpoint,diff,failed_verification,repair_attempt,passing_verification,rewind_references",
        output,
    ));
    assert!(!structured_json_satisfies_field_selector(
        "checkpoint,diff,missing_field",
        output,
    ));
}

#[test]
fn structured_selector_does_not_use_visible_text_fallback() {
    let output =
        r#"{"extra":{"checkpoint":{"status":"planned"}},"text":"diff={\"status\":\"planned\"}"}"#;

    assert!(!structured_json_satisfies_field_selector(
        "checkpoint,diff",
        output,
    ));
    assert!(!structured_json_satisfies_field_selector("text", output));
}

#[test]
fn exact_machine_selector_is_bounded_and_domain_neutral() {
    assert_eq!(
        exact_machine_field_selector("datetime, timezone;title"),
        Some(vec![
            "datetime".to_string(),
            "timezone".to_string(),
            "title".to_string()
        ])
    );
    assert_eq!(
        exact_machine_field_selector("quote.price_usd,quote.price_usd"),
        Some(vec!["quote.price_usd".to_string()])
    );
    assert!(exact_machine_field_selector("text").is_none());
    assert!(exact_machine_field_selector("items.*").is_none());
}
