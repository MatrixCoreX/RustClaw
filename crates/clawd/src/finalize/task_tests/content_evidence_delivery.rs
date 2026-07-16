use super::super::task_content_evidence_delivery::{
    backfill_content_evidence_file_delivery_from_journal, backfill_file_delivery_token_from_journal,
};
use super::route_result;

#[test]
fn content_evidence_file_delivery_backfills_read_range_from_journal_excerpt() {
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.delivery_required = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "deliver config content");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_range","mode":"head","requested_n":80,"excerpt":"1|[app]\n2|name = \"RustClaw NL Fixture\"\n3|mode = \"test\"","path":"/tmp/app_config.toml"}}"#,
        ));

    let mut answer_text =
        "RustClaw fixture config summary.\n\nFILE:/tmp/app_config.toml".to_string();
    let mut answer_messages = vec![
        "RustClaw fixture config summary.".to_string(),
        "FILE:/tmp/app_config.toml".to_string(),
    ];

    assert!(backfill_content_evidence_file_delivery_from_journal(
        &route,
        &journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.starts_with("[app]\nname = \"RustClaw NL Fixture\"\nmode = \"test\""));
    assert!(answer_text.contains("RustClaw fixture config summary."));
    assert!(answer_text.contains("FILE:/tmp/app_config.toml"));
    assert_eq!(
        answer_messages.first().map(String::as_str),
        Some("[app]\nname = \"RustClaw NL Fixture\"\nmode = \"test\"")
    );
}

#[test]
fn content_evidence_file_delivery_backfills_missing_file_token_from_read_range_path() {
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.delivery_required = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "deliver config content");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","mode":"head","requested_n":80,"excerpt":"1|[app]\n2|name = \"RustClaw NL Fixture\"\n3|mode = \"test\"","resolved_path":"/tmp/app_config.toml"}}"#,
        ));

    let mut answer_text = "[app]\nname = \"RustClaw NL Fixture\"\nmode = \"test\"\n\nRustClaw fixture config summary.".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(backfill_content_evidence_file_delivery_from_journal(
        &route,
        &journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.starts_with("FILE:/tmp/app_config.toml"));
    assert!(answer_text.contains("[app]\nname = \"RustClaw NL Fixture\"\nmode = \"test\""));
    assert_eq!(
        answer_messages.first().map(String::as_str),
        Some("FILE:/tmp/app_config.toml")
    );
}

#[test]
fn file_delivery_backfills_missing_file_token_from_write_output_path() {
    let mut route = route_result();
    route.delivery_required = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "write and deliver json");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/tmp/manual_meta.json","bytes_written":31}}"#,
        ));

    let mut answer_text = "path=/tmp/manual_meta.json".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(backfill_file_delivery_token_from_journal(
        &route,
        &journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.starts_with("FILE:/tmp/manual_meta.json"));
    assert!(answer_text.contains("path=/tmp/manual_meta.json"));
    assert_eq!(
        answer_messages.first().map(String::as_str),
        Some("FILE:/tmp/manual_meta.json")
    );
}
