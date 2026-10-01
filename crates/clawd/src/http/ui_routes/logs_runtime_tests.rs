use std::io::BufRead;

use super::{
    available_log_file_names_page, is_log_file_name, open_usage_log_window,
    select_available_log_file,
};

#[test]
fn discovers_existing_logs_without_accepting_lock_or_unrelated_files() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-log-discovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create fixture directory");
    for file_name in [
        "clawd.log",
        "model_io.log.2026-07-28",
        "notes.txt",
        "model_io.log.lock",
    ] {
        std::fs::write(root.join(file_name), file_name).expect("write fixture file");
    }
    std::fs::create_dir(root.join("nested.log")).expect("create fixture subdirectory");

    let page = available_log_file_names_page(&root, None, 100).expect("discover log files");

    assert_eq!(page.files, vec!["clawd.log", "model_io.log.2026-07-28"]);
    assert_eq!(page.total, 2);
    assert!(!page.has_more);
    std::fs::remove_dir_all(root).expect("remove fixture directory");
}

#[test]
fn selects_only_a_name_returned_by_log_discovery() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-log-selection-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create fixture directory");
    std::fs::write(root.join("clawd.log"), "clawd").expect("write clawd log");
    std::fs::write(root.join("webd.log"), "webd").expect("write webd log");

    assert_eq!(
        select_available_log_file(&root, None)
            .expect("default selection")
            .as_deref(),
        Some("clawd.log")
    );
    assert_eq!(
        select_available_log_file(&root, Some("webd.log"))
            .expect("named selection")
            .as_deref(),
        Some("webd.log")
    );
    assert_eq!(
        select_available_log_file(&root, Some("../clawd.log")).expect("invalid name result"),
        None
    );
    assert!(is_log_file_name("runtime.log"));
    assert!(is_log_file_name("runtime.log.1"));
    assert!(!is_log_file_name("runtime.log.lock"));
    std::fs::remove_dir_all(root).expect("remove fixture directory");
}

#[test]
fn log_file_directory_is_cursor_paged_with_bounded_candidates() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-log-page-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create fixture directory");
    for index in 0..7 {
        std::fs::write(
            root.join(format!("runtime-{index:02}.log")),
            index.to_string(),
        )
        .expect("write fixture log");
    }

    let first = available_log_file_names_page(&root, None, 3).expect("first page");
    assert_eq!(
        first.files,
        vec!["runtime-00.log", "runtime-01.log", "runtime-02.log"]
    );
    assert_eq!(first.total, 7);
    assert!(first.has_more);
    assert_eq!(first.next_cursor.as_deref(), Some("runtime-02.log"));

    let second =
        available_log_file_names_page(&root, first.next_cursor.as_deref(), 3).expect("second page");
    assert_eq!(
        second.files,
        vec!["runtime-03.log", "runtime-04.log", "runtime-05.log"]
    );
    assert!(second.has_more);

    let third =
        available_log_file_names_page(&root, second.next_cursor.as_deref(), 3).expect("third page");
    assert_eq!(third.files, vec!["runtime-06.log"]);
    assert!(!third.has_more);
    std::fs::remove_dir_all(root).expect("remove fixture directory");
}

#[test]
fn usage_log_window_discards_partial_prefix_and_reports_truncation() {
    let path = std::env::temp_dir().join(format!(
        "agent-runtime-usage-window-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, b"first-record\nsecond-record\nthird-record\n")
        .expect("write usage fixture");

    let (reader, window) = open_usage_log_window(&path, 25).expect("open bounded tail");
    let lines = reader
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .expect("read complete lines");

    assert_eq!(lines, vec!["third-record"]);
    assert!(window.truncated_before);
    assert_eq!(window.total_bytes, 40);
    assert_eq!(window.max_bytes, 25);
    assert_eq!(window.scanned_bytes, 13);
    std::fs::remove_file(path).expect("remove usage fixture");
}

#[test]
fn usage_log_window_keeps_entire_small_file() {
    let path = std::env::temp_dir().join(format!(
        "agent-runtime-usage-window-small-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, b"first\nsecond\n").expect("write usage fixture");

    let (reader, window) = open_usage_log_window(&path, 1024).expect("open full log");
    let lines = reader
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .expect("read complete lines");

    assert_eq!(lines, vec!["first", "second"]);
    assert!(!window.truncated_before);
    assert_eq!(window.scanned_bytes, window.total_bytes);
    std::fs::remove_file(path).expect("remove usage fixture");
}
