use super::{available_log_file_names, is_log_file_name, select_available_log_file};

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

    let files = available_log_file_names(&root).expect("discover log files");

    assert_eq!(files, vec!["clawd.log", "model_io.log.2026-07-28"]);
    std::fs::remove_dir_all(root).expect("remove fixture directory");
}

#[test]
fn selects_only_a_name_returned_by_log_discovery() {
    let files = vec!["clawd.log".to_string(), "webd.log".to_string()];

    assert_eq!(
        select_available_log_file(&files, None).as_deref(),
        Some("clawd.log")
    );
    assert_eq!(
        select_available_log_file(&files, Some("webd.log")).as_deref(),
        Some("webd.log")
    );
    assert_eq!(
        select_available_log_file(&files, Some("../clawd.log")),
        None
    );
    assert!(is_log_file_name("runtime.log"));
    assert!(is_log_file_name("runtime.log.1"));
    assert!(!is_log_file_name("runtime.log.lock"));
}
