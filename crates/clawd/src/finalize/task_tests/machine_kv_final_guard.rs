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
fn requested_machine_kv_summary_final_guard_replaces_scalar_path_when_explicit_pair_is_observed() {
    let prompt = "只读定位 AGENTS.md 中包含 check_no_nl_hardmatch.py 的规则行或邻近行，最终只保留机器字段 no_hardmatch_guard=check_no_nl_hardmatch.py；不要修改文件。";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "AGENTS.md".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-kv-path-scalar", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"grep_text","matches":[{"line":244,"path":"AGENTS.md","text":"run `python3 scripts/check_no_nl_hardmatch.py` after boundary changes"}],"query":"check_no_nl_hardmatch.py","results":["AGENTS.md"],"root":"AGENTS.md"},"text":"AGENTS.md"}"#,
        ));
    let mut answer_text = "AGENTS.md".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(answer_text, "no_hardmatch_guard=check_no_nl_hardmatch.py");
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
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
fn requested_machine_kv_summary_final_guard_restores_web_search_candidates_over_scalar_summary() {
    let prompt =
        "Search the web for Rust async tutorial top_k=3 and return titles plus source domains.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-machine-kv-web-search-candidate-restore",
        "ask",
        prompt,
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "web_search_extract",
            r#"{"extra":{"action":"search_extract","top_k":3,"candidates":[{"title":"Introduction - Asynchronous Programming in Rust","source":"rust-lang.github.io","url":"https://rust-lang.github.io/async-book/"},{"title":"Fundamentals of Asynchronous Programming: Async, Await ... - Learn Rust","source":"doc.rust-lang.org","url":"https://doc.rust-lang.org/book/ch17-00-async-await.html"},{"title":"Introduction - Asynchronous Programming in Rust","source":"rust-lang.github.io","url":"https://rust-lang.github.io/async-book/part-guide/intro.html"}]},"text":"{\"candidates\":[{\"title\":\"must_not_parse_text\",\"source\":\"bad.example\"}]}"}"#,
        ));
    let mut answer_text = "source=doc.rust-lang.org top_k=3".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text
        .contains("Introduction - Asynchronous Programming in Rust - rust-lang.github.io"));
    assert!(answer_text.contains(
        "Fundamentals of Asynchronous Programming: Async, Await ... - Learn Rust - doc.rust-lang.org"
    ));
    assert!(!answer_text.contains("must_not_parse_text"));
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
fn requested_machine_kv_summary_final_guard_preserves_verified_config_summary() {
    let prompt = "只预览把 configs/config.toml 里的 llm.selected_vendor 改成 minimax，不要写入；然后读取当前 llm.selected_vendor，并运行配置风险检查；回答预览是否会改变、当前值和是否有明显风险。";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-config-verified-summary", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "config_basic",
            r#"{"extra":{"action":"read_fields","path":"configs/config.toml","results":[{"field_path":"llm.selected_vendor","value":"minimax","value_text":"minimax"}]}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "config_basic",
            r#"{"extra":{"action":"guard_config","path":"configs/config.toml","valid":false,"risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"]}}"#,
        ));
    journal.record_answer_verifier_summary(crate::answer_verifier::AnswerVerifierOut {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.95,
    });
    let mut answer_text = "仅预览，未写入磁盘；llm.selected_vendor 当前值是 minimax，目标值也是 minimax，所以预览不会改变该字段。配置风险检查发现 2 项既有风险：tools.allow_sudo=true 和 tools.allow_path_outside_workspace=true。"
        .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("预览不会改变"));
    assert_ne!(answer_text, "llm.selected_vendor");
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
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

#[test]
fn requested_machine_kv_summary_does_not_recover_required_content_gap() {
    let prompt =
        "给 calc_core.py 增加 mul(a,b)，更新测试后只输出 changed_files、test_command、test_status、functions。";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-kv-content-gap", "ask", prompt);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string(), "field_value".to_string()],
        answer_incomplete_reason:
            "requested code mutation was not observed after reading old add/sub content".to_string(),
        should_retry: true,
        retry_instruction: "modify files, collect post-write excerpts, and rerun tests".to_string(),
        confidence: 0.96,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            r#"{"changed_files":[],"test_command":"python3 test_calc_core.py","test_status":"OK","functions":["add","sub"]}"#,
        ));
    let mut answer_text =
        r#"{"changed_files":[],"test_command":"python3 test_calc_core.py","test_status":"OK","functions":["add","sub"]}"#
            .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!recover_requested_machine_kv_summary_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        true,
    ));

    assert!(journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| {
            !summary.pass
                && summary
                    .missing_evidence_fields
                    .iter()
                    .any(|field| field == "content_excerpt")
        }));
    assert_eq!(
        answer_text,
        r#"{"changed_files":[],"test_command":"python3 test_calc_core.py","test_status":"OK","functions":["add","sub"]}"#
    );
}

#[test]
fn requested_machine_kv_summary_force_patches_archive_db_json_instead_of_scalar_replace() {
    let prompt = "Return required machine field user_version.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-archive-db-force-merge", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "archive_basic",
            r#"{"extra":{"action":"list","archive":"scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","entries":[{"name":"notes.txt","kind":"file"},{"name":"nested/config.ini","kind":"file"}]}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "archive_basic",
            r#"{"extra":{"action":"read","archive":"scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","member":"notes.txt","content":"fixture archive notes","content_excerpt":"fixture archive notes"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "db_basic",
            r#"{"extra":{"action":"list_tables","db_path":"scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite","table_count":3,"tables":["orders","service_logs","users"],"field_value":{"table_count":3,"tables":["orders","service_logs","users"]}}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "db_basic",
            r#"{"extra":{"action":"user_version","db_path":"scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite","field_value":{"user_version":7},"user_version":7},"text":"user_version=7"}"#,
        ));
    let mut answer_text = serde_json::json!({
        "archive": {
            "entries": ["notes.txt", "nested/config.ini"],
            "member": {"name": "notes.txt", "content": "fixture archive notes"}
        },
        "database": {
            "tables": ["orders", "service_logs", "users"]
        }
    })
    .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(recover_requested_machine_kv_summary_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        true,
    ));

    let value: serde_json::Value = serde_json::from_str(&answer_text).expect("patched json");
    assert_eq!(
        value
            .pointer("/archive/entries/0")
            .and_then(serde_json::Value::as_str),
        Some("notes.txt")
    );
    assert_eq!(
        value
            .pointer("/archive/member/content")
            .and_then(serde_json::Value::as_str),
        Some("fixture archive notes")
    );
    assert_eq!(
        value
            .pointer("/database/tables/1")
            .and_then(serde_json::Value::as_str),
        Some("service_logs")
    );
    assert_eq!(
        value
            .get("user_version")
            .and_then(serde_json::Value::as_i64),
        Some(7)
    );
    assert_ne!(answer_text, "user_version=7");
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_final_guard_restores_path_fact_over_filename_marker() {
    let prompt = "rustclaw.service";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-machine-kv-path-fact-final",
        "ask",
        prompt,
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/home/guagua/rustclaw/rustclaw.service","size_bytes":769},"path":"/home/guagua/rustclaw/rustclaw.service"}],"include_missing":true}}"#,
        ));
    let mut answer_text = "rustclaw.service".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("message_key=clawd.msg.path_fact.observed"));
    assert!(answer_text.contains("reason_code=path_fact_observed"));
    assert!(answer_text.contains("exists=true"));
    assert!(answer_text.contains("path=/home/guagua/rustclaw/rustclaw.service"));
    assert!(answer_text.contains("kind=file"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}
