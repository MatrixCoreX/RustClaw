use super::{
    analyze_log_file, candidate_priority, execute, log_level_from_line, sanitize_match_line,
};
use serde_json::json;
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

#[test]
fn analysis_keeps_recovery_lines_visible_even_when_info_level() {
    let dir = std::env::temp_dir().join(format!(
        "rustclaw-log-analyze-recovery-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("app.log");
    std::fs::write(
        &path,
        "2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata\n2026-04-01 10:08:46 INFO provider retry succeeded on second attempt\n",
    )
    .expect("write log");
    let keywords = ["error", "timeout", "retry", "succeeded"]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let analysis =
        analyze_log_file(&path, path.display().to_string(), &keywords, 20).expect("analysis");

    assert_eq!(analysis.keyword_counts.get("retry"), Some(&1));
    assert_eq!(analysis.recovery_counts.get("retry"), Some(&1));
    assert_eq!(analysis.recovery_counts.get("succeeded"), Some(&1));
    assert!(analysis
        .recent_recovery_lines
        .iter()
        .any(|line| line.contains("provider retry succeeded")));
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir(dir);
}

#[test]
fn execute_returns_structured_extra_alongside_legacy_text_json() {
    let dir = std::env::temp_dir().join(format!(
        "rustclaw-log-analyze-extra-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("app.log");
    std::fs::write(
        &path,
        "2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata\n",
    )
    .expect("write log");

    let (text, extra) = execute(json!({
        "path": path.to_string_lossy(),
        "keywords": ["error", "timeout"],
        "max_matches": 5
    }))
    .expect("execute log analyze");

    let text_value: serde_json::Value = serde_json::from_str(&text).expect("text json");
    assert_eq!(text_value, extra);
    assert_eq!(
        extra.get("action").and_then(|value| value.as_str()),
        Some("analyze_log")
    );
    assert_eq!(
        extra.get("total_lines").and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        extra
            .pointer("/keyword_counts/error")
            .and_then(|value| value.as_u64()),
        Some(1)
    );

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir(dir);
}
