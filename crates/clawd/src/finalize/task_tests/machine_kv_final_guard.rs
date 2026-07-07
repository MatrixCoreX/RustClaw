use super::*;

#[test]
fn requested_machine_kv_summary_final_guard_preserves_transform_markdown_table() {
    let prompt = r#"Sort [{"name":"alpha","score":7},{"name":"beta","score":12}] by score and return a markdown table."#;
    let mut route = route_result(crate::AskMode::planner_execute_plain());
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
    let mut route = route_result(crate::AskMode::planner_execute_plain());
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
    let mut route = route_result(crate::AskMode::planner_execute_plain());
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
fn requested_machine_kv_summary_final_guard_preserves_weather_query_fields() {
    let prompt = "查询北京当前天气，只返回 location、temperature、weather_code。";
    let mut route = route_result(crate::AskMode::planner_execute_plain());
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
    let mut route = route_result(crate::AskMode::planner_execute_plain());
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
