use super::*;

#[test]
fn runner_bin_name_normalizes_underscores_and_appends_suffix() {
    assert_eq!(runner_bin_name("fs_search").unwrap(), "fs-search-skill");
    assert_eq!(runner_bin_name("rss_fetch").unwrap(), "rss-fetch-skill");
}

#[test]
fn runner_bin_name_passes_through_when_already_suffixed() {
    assert_eq!(runner_bin_name("demo-skill").unwrap(), "demo-skill");
    assert_eq!(runner_bin_name("demo_skill").unwrap(), "demo-skill");
}

#[test]
fn runner_bin_name_rejects_empty_or_path_like() {
    assert!(runner_bin_name("").is_err());
    assert!(runner_bin_name("   ").is_err());
    assert!(runner_bin_name("a/b").is_err());
    assert!(runner_bin_name("a\\b").is_err());
}

#[tokio::test]
async fn run_child_skill_times_out_and_kills_child() {
    // `yes` runs until killed, so it reliably exercises the timeout branch
    // without depending on executing a just-written temp script.
    let Some(child) = ["/usr/bin/yes", "/bin/yes"]
        .into_iter()
        .find(|path| Path::new(path).exists())
    else {
        eprintln!("skipping timeout assertion: no `yes` executable found");
        return;
    };

    let result = run_child_skill(child, "ignored", Duration::from_millis(150)).await;
    assert!(
        matches!(result, Err(ref e) if e == "child skill timeout"),
        "expected timeout, got {:?}",
        result
    );
}

#[tokio::test]
async fn run_child_skill_reports_nonzero_exit() {
    let result = run_child_skill("/bin/false", "ignored", Duration::from_secs(2)).await;
    assert!(matches!(result, Err(ref e) if e.starts_with("child exited with")));
}

#[tokio::test]
async fn run_child_skill_returns_first_stdout_line() {
    let result = run_child_skill("/bin/cat", "hello-from-stdin", Duration::from_secs(2))
        .await
        .expect("cat should echo stdin");
    assert_eq!(result, "hello-from-stdin");
}
