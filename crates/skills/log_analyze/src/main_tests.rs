use super::{analyze_log_file, candidate_priority, log_level_from_line, sanitize_match_line};
use std::path::Path;

#[test]
fn candidate_priority_prefers_operational_logs_over_model_io() {
    assert!(
        candidate_priority(Path::new("clawd.log")) > candidate_priority(Path::new("model_io.log"))
    );
    assert!(
        candidate_priority(Path::new("telegramd.log"))
            > candidate_priority(Path::new("model_io.log"))
    );
}

#[test]
fn sanitize_match_line_truncates_oversized_lines() {
    let long = "a".repeat(400);
    let out = sanitize_match_line(&long, 32);
    assert!(out.len() < long.len());
    assert!(out.ends_with("...(truncated)"));
}

#[test]
fn detects_standard_log_levels_as_structured_notable_lines() {
    assert_eq!(
        log_level_from_line("2026-04-01 10:02:20 WARN upstream latency increased"),
        Some("warn")
    );
    assert_eq!(
        log_level_from_line("2026-04-01 10:08:44 ERROR provider timeout"),
        Some("error")
    );
}

#[test]
fn default_analysis_keeps_warn_latency_visible() {
    let dir =
        std::env::temp_dir().join(format!("rustclaw-log-analyze-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("app.log");
    std::fs::write(
        &path,
        "2026-04-01 10:00:00 INFO boot\n2026-04-01 10:02:20 WARN upstream latency increased to 820ms\n",
    )
    .expect("write log");
    let keywords = [
        "error",
        "warn",
        "warning",
        "failed",
        "timeout",
        "panic",
        "latency",
        "queue full",
        "unauthorized",
    ]
    .into_iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();

    let analysis =
        analyze_log_file(&path, path.display().to_string(), &keywords, 20).expect("analysis");

    assert_eq!(analysis.level_counts.get("warn"), Some(&1));
    assert_eq!(analysis.keyword_counts.get("warn"), Some(&1));
    assert_eq!(analysis.keyword_counts.get("latency"), Some(&1));
    assert!(analysis
        .recent_notable_lines
        .iter()
        .any(|line| line.contains("latency increased")));
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir(dir);
}
