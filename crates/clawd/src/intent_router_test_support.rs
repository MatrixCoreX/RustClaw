pub(super) fn make_temp_workspace_with_child(
    test_name: &str,
    child_name: &str,
) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_intent_router_{test_name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join(child_name)).expect("create child directory");
    root
}
