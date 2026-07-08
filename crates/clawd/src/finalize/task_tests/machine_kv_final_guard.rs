use super::*;

#[test]
fn requested_machine_kv_summary_final_guard_preserves_terminal_scalar_respond() {
    let prompt =
        "Count entries under scripts/nl_tests/fixtures/device_local and return only the digit.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.resolved_intent =
        "Count top-level directories under scripts/nl_tests/fixtures/device_local.".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-kv-scalar", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"inventory_dir","counts":{"dirs":5,"files":0,"total":5},"dirs_only":true,"path":"scripts/nl_tests/fixtures/device_local"},"text":"{\"action\":\"inventory_dir\"}"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2", "respond", "5",
        ));
    let mut answer_text = "5".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(answer_text, "5");
    assert_eq!(answer_messages, vec!["5".to_string()]);
    assert_eq!(journal.final_answer.as_deref(), Some("5"));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_observed_empty_string_scalar() {
    let prompt = "Read ./Cargo.toml workspace.package.repository and output only the value.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.resolved_intent =
        "Read ./Cargo.toml workspace.package.repository and output only the value.".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "./Cargo.toml".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-kv-empty-scalar", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "config_basic",
            r#"{"extra":{"action":"extract_field","exists":true,"field_path":"workspace.package.repository","path":"/home/guagua/rustclaw/Cargo.toml","resolved_field_path":"workspace.package.repository","value":"","value_text":"","value_type":"string"},"text":"{\"action\":\"extract_field\"}"}"#,
        ));
    let mut answer_text = "\"\"".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(answer_text, "\"\"");
    assert_eq!(answer_messages, vec!["\"\"".to_string()]);
    assert_eq!(journal.final_answer.as_deref(), Some("\"\""));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_transform_markdown_table() {
    let prompt = r#"Sort [{"name":"alpha","score":7},{"name":"beta","score":12}] by score and return a markdown table."#;
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.resolved_intent = "name".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-machine-kv-transform-table",
        "ask",
        prompt,
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "transform",
            r#"{"extra":{"action":"transform_data"},"text":"{\"output\":\"| name | score |\\n| --- | --- |\\n| beta | 12 |\\n| alpha | 7 |\",\"result\":[{\"name\":\"beta\",\"score\":12},{\"name\":\"alpha\",\"score\":7}]}"}"#,
        ));
    let mut answer_text =
        "| name | score |\n| --- | --- |\n| beta | 12 |\n| alpha | 7 |".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("beta"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_transform_json_array() {
    let prompt = r#"Filter records [{"name":"a","ok":true},{"name":"b","ok":false},{"name":"c","ok":true}] where ok=true and return JSON array."#;
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.resolved_intent = "name ok".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-machine-kv-transform-json-array",
        "ask",
        prompt,
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "transform",
            r#"{"extra":{"action":"transform_data"},"text":"{\"output\":[{\"name\":\"a\",\"ok\":true},{\"name\":\"c\",\"ok\":true}],\"result\":[{\"name\":\"a\",\"ok\":true},{\"name\":\"c\",\"ok\":true}]}"}"#,
        ));
    let mut answer_text = r#"[{"name":"a","ok":true},{"name":"c","ok":true}]"#.to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains(r#""name":"c""#));
    assert!(answer_text.trim_start().starts_with('['));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_web_search_listing() {
    let prompt = "Search the web for Rust async tutorial top_k=3 and return titles only.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.resolved_intent = "capability_ref=web.search_results top_k=3 titles only".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-machine-kv-web-search-listing",
        "ask",
        prompt,
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "web_search_extract",
            r#"{"extra":{"action":"search_extract","top_k":3,"candidates":[{"title":"tdejager/tutorial_bot","source":"github.com","url":"https://github.com/tdejager/tutorial_bot"},{"title":"volodymyrd/rust-async-tutorial","source":"github.com","url":"https://github.com/volodymyrd/rust-async-tutorial"}]}}"#,
        ));
    let mut answer_text = "tdejager/tutorial_bot\nvolodymyrd/rust-async-tutorial".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("rust-async-tutorial"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_workspace_grounded_summary() {
    let prompt = "List clawd related log files, read clawd.run.log tail, then summarize status.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_hint = "logs/clawd.run.log".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-workspace-grounded-final", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"inventory_dir","names_by_kind":{"files":["clawd.log","clawd.run.log","clawd.out"]},"path":"/home/guagua/rustclaw/logs"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_range","mode":"tail","requested_n":20,"path":"/home/guagua/rustclaw/logs/clawd.run.log","excerpt":"468|INFO task_call executor_step_execute\n469|INFO task_call skill_dispatch"}}"#,
        ));
    let mut answer_text = "与 clawd 相关的文件包括 clawd.log、clawd.run.log、clawd.out；clawd.run.log 尾部都是 INFO 级 task_call 执行日志，没有 ERROR 或 WARN，服务更像正常启动并持续处理任务。"
        .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("服务更像正常启动"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert!(journal.final_answer.as_deref() != Some("clawd.run.log"));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_delivery_file_token() {
    let prompt = "Create a text file in tmp/notes.txt, write content, and send the file.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.resolved_intent =
        "workspace_root=/home/guagua/rustclaw create tmp/notes.txt delivery_required=true"
            .to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-delivery-file-token-kv", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"make_dir","path":"/home/guagua/rustclaw/tmp","resolved_path":"/home/guagua/rustclaw/tmp"},"text":"created directory /home/guagua/rustclaw/tmp"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "exit=0 command=printf content > tmp/notes.txt",
        ));
    let mut answer_text = "FILE:/home/guagua/rustclaw/tmp/notes.txt".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(answer_text, "FILE:/home/guagua/rustclaw/tmp/notes.txt");
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
    assert_ne!(
        journal.final_answer.as_deref(),
        Some("workspace_root=/home/guagua/rustclaw")
    );
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_weather_query_fields() {
    let prompt = "查询北京当前天气，只返回 location、temperature、weather_code。";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.resolved_intent = "capability_ref=weather.current location=Beijing".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-weather-machine-kv", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "weather",
            r#"{"extra":{"location":"北京","temperature":25.4,"weather_code":"多云","weather_code_raw":3}}"#,
        ));
    let mut answer_text = "location=北京\ntemperature=25.4\nweather_code=多云".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(
        answer_text,
        "location=北京\ntemperature=25.4\nweather_code=多云"
    );
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_publishable_command_summary() {
    let prompt = "Run pwd, inspect the local port, and answer with the working directory and whether a port is visible.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-command-summary-final", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "run_cmd",
            r#"{"extra":{"action":"run_cmd","command":"pwd","command_output":"/home/guagua/rustclaw"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "process_basic",
            r#"{"extra":{"action":"port_list","port":8787,"process":"clawd","pid":892143}}"#,
        ));
    let mut answer_text = "Working directory: /home/guagua/rustclaw. A clawd-related process is running, and port 8787 is visible.".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("clawd-related process"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
    assert_ne!(journal.final_answer.as_deref(), Some("port=8787"));
}
