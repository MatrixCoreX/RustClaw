use super::*;
use std::path::PathBuf;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 2);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.fs_search.execution_failed");
    assert_eq!(extra["retryable"], false);
}

fn unique_temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "agent-runtime-fs-search-{name}-{}",
        std::process::id()
    ))
}

#[test]
fn find_name_reaches_nested_prompt_paths_with_explicit_depth() {
    let root = unique_temp_dir("nested-prompt");
    let nested = root.join("prompts/layers/overlays");
    std::fs::create_dir_all(&nested).expect("create nested dir");
    std::fs::write(nested.join("intent_normalizer_prompt.md"), "# prompt\n")
        .expect("write prompt file");

    let out = execute(json!({
        "action": "find_name",
        "pattern": "intent_normalizer_prompt",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 8,
        "max_results": 10
    }))
    .expect("find_name succeeds");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    assert_eq!(out.get("truncated").and_then(Value::as_bool), Some(false));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array");
    assert!(
        results.iter().any(|v| v
            .as_str()
            .is_some_and(|s| s.ends_with("prompts/layers/overlays/intent_normalizer_prompt.md"))),
        "results={results:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_reports_result_limit_truncation() {
    let root = unique_temp_dir("find-name-truncated");
    std::fs::create_dir_all(&root).expect("create root");
    for name in ["match-a.txt", "match-b.txt", "match-c.txt"] {
        std::fs::write(root.join(name), "match\n").expect("write fixture");
    }

    let out = execute(json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy().to_string(),
        "max_results": 2
    }))
    .expect("find_name succeeds");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(2));
    assert_eq!(out.get("returned_count").and_then(Value::as_u64), Some(2));
    assert_eq!(out.get("result_limit").and_then(Value::as_u64), Some(2));
    assert_eq!(out.get("truncated").and_then(Value::as_bool), Some(true));
    assert_eq!(
        out.get("results").and_then(Value::as_array).map(Vec::len),
        Some(2)
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_directory_target_uses_kind_filter() {
    let root = unique_temp_dir("dir-budget");
    let noisy = root.join("a_many_files");
    let target = root.join("z_parent/bundle_src");
    std::fs::create_dir_all(&noisy).expect("create noisy dir");
    std::fs::create_dir_all(&target).expect("create target dir");
    std::fs::write(root.join("z_parent/readme.txt"), "nearby file\n").expect("write sibling file");
    for idx in 0..8 {
        std::fs::write(noisy.join(format!("noise_{idx}.txt")), "noise\n")
            .expect("write noise file");
    }

    let out = execute(json!({
        "action": "find_name",
        "pattern": "bundle_src",
        "root": root.to_string_lossy().to_string(),
        "target_kind": "directory",
        "max_depth": 4,
        "max_results": 5
    }))
    .expect("find_name succeeds");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array");
    assert!(
        results.iter().any(|v| v
            .as_str()
            .is_some_and(|s| s.ends_with("z_parent/bundle_src"))),
        "results={results:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_accepts_multiple_patterns_and_file_filter_alias() {
    let root = unique_temp_dir("multi-pattern");
    std::fs::create_dir_all(root.join("audio_dir")).expect("create audio dir");
    std::fs::write(root.join("audio.toml"), "").expect("write audio config");
    std::fs::write(root.join("image.toml"), "").expect("write image config");
    std::fs::write(root.join("stock.toml"), "").expect("write unrelated config");

    let out = execute(json!({
        "action": "find_name",
        "patterns": ["*audio*", "*image*"],
        "files_only": true,
        "root": root.to_string_lossy().to_string(),
        "max_depth": 2,
        "max_results": 10
    }))
    .expect("find_name succeeds with patterns");

    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(results.iter().any(|path| path.ends_with("audio.toml")));
    assert!(results.iter().any(|path| path.ends_with("image.toml")));
    assert!(!results.iter().any(|path| path.ends_with("audio_dir")));
    assert!(!results.iter().any(|path| path.ends_with("stock.toml")));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_accepts_entry_name_alias() {
    let root = unique_temp_dir("entry-name-alias");
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).expect("create nested dir");
    std::fs::write(nested.join("config.ini"), "").expect("write config");
    std::fs::write(root.join("config.txt"), "").expect("write sibling");

    let out = execute(json!({
        "action": "find_name",
        "entry_name": "config.ini",
        "target_kind": "file",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 3,
        "max_results": 10
    }))
    .expect("find_name succeeds with entry_name alias");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert!(results[0].ends_with("nested/config.ini"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_exact_basename_avoids_stem_contains_match() {
    let root = unique_temp_dir("exact-basename");
    let _ = std::fs::remove_dir_all(&root);
    let exact_dir = root.join("case_only");
    let fuzzy_dir = root.join("fuzzy_top3");
    std::fs::create_dir_all(&exact_dir).expect("create exact dir");
    std::fs::create_dir_all(&fuzzy_dir).expect("create fuzzy dir");
    std::fs::write(exact_dir.join("Report.MD"), "").expect("write exact report");
    std::fs::write(fuzzy_dir.join("abcd_report.md"), "").expect("write fuzzy report");

    let out = execute(json!({
        "action": "find_name",
        "pattern": "Report.MD",
        "exact": true,
        "target_kind": "file",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 3,
        "max_results": 10
    }))
    .expect("find_name succeeds with exact basename");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert!(results[0].ends_with("case_only/Report.MD"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn search_success_exposes_absolute_workspace_root() {
    let root = workspace_root();
    let out = execute(json!({
        "action": "find_name",
        "pattern": "Cargo.toml",
        "exact": true,
        "target_kind": "file",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 1,
        "max_results": 10
    }))
    .expect("find_name succeeds");

    assert_eq!(
        out.get("workspace_root").and_then(Value::as_str),
        Some(root.to_string_lossy().as_ref())
    );
    assert!(root.is_absolute());
    assert_eq!(out.get("root").and_then(Value::as_str), Some(""));
    assert!(out
        .get("results")
        .and_then(Value::as_array)
        .is_some_and(|results| results == &[json!("Cargo.toml")]));
}

#[test]
fn find_name_finds_shallow_files_alongside_deep_entries() {
    let root = unique_temp_dir("shallow-before-deep");
    let deep = root.join("aaa_deep");
    std::fs::create_dir_all(&deep).expect("create deep dir");
    for idx in 0..8 {
        std::fs::write(deep.join(format!("noise-{idx}.txt")), "").expect("write noise");
    }
    std::fs::write(root.join("start-all-bin.sh"), "#!/usr/bin/env bash\n")
        .expect("write shallow script");

    let out = execute(json!({
        "action": "find_name",
        "pattern": "start-all-bin.sh",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 8,
        "max_results": 10
    }))
    .expect("find_name succeeds");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array");
    assert!(results
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s.ends_with("start-all-bin.sh"))));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_expands_simple_alternation_pattern() {
    let root = unique_temp_dir("alternation-pattern");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("speech.toml"), "").expect("write speech config");
    std::fs::write(root.join("photo.toml"), "").expect("write photo config");

    let out = execute(json!({
        "action": "find_name",
        "pattern": "*(speech|photo)*",
        "files_only": true,
        "root": root.to_string_lossy().to_string(),
        "max_depth": 1,
        "max_results": 10
    }))
    .expect("find_name succeeds with alternation");

    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(results.iter().any(|path| path.ends_with("speech.toml")));
    assert!(results.iter().any(|path| path.ends_with("photo.toml")));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_ext_respects_optional_name_pattern() {
    let root = unique_temp_dir("find-ext-pattern");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("execution_intent_routing_repair_plan_20260509.md"),
        "",
    )
    .expect("write target plan");
    std::fs::write(root.join("builtin_skill_capability_governance_plan.md"), "")
        .expect("write unrelated plan");
    std::fs::write(root.join("execution_intent_trace.txt"), "").expect("write non-md file");

    let out = execute(json!({
        "action": "find_ext",
        "ext": "md",
        "pattern": "*execution_intent*.md",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 1,
        "max_results": 10
    }))
    .expect("find_ext succeeds with pattern");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert!(results[0].ends_with("execution_intent_routing_repair_plan_20260509.md"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_ext_accepts_extension_alias_array_and_pattern() {
    let root = unique_temp_dir("find-ext-alias-array");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("clawd.log.md"), "").expect("write md target");
    std::fs::write(root.join("agent-log.txt"), "").expect("write txt target");
    std::fs::write(root.join("agent-log.toml"), "").expect("write non-target extension");
    std::fs::write(root.join("notes.md"), "").expect("write non-target name");

    let out = execute(json!({
        "action": "find_ext",
        "ext_filter": ["md", ".txt"],
        "query": "log",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 1,
        "max_results": 10
    }))
    .expect("find_ext succeeds with extension aliases");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(2));
    let exts = out
        .get("exts")
        .and_then(Value::as_array)
        .expect("exts array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(exts, vec!["md", "txt"]);
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(results.iter().any(|path| path.ends_with("clawd.log.md")));
    assert!(results.iter().any(|path| path.ends_with("agent-log.txt")));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_ext_respects_max_results_across_subdirectories() {
    let root = unique_temp_dir("find-ext-max-results");
    for dir in ["a", "b", "c"] {
        std::fs::create_dir_all(root.join(dir)).expect("create nested dir");
        std::fs::write(root.join(dir).join(format!("{dir}.toml")), "").expect("write config");
    }

    let out = execute(json!({
        "action": "find_ext",
        "ext": "toml",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 2,
        "max_results": 2
    }))
    .expect("find_ext succeeds");

    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array");
    assert_eq!(out.get("count").and_then(Value::as_u64), Some(2));
    assert_eq!(out.get("truncated").and_then(Value::as_bool), Some(true));
    assert_eq!(results.len(), 2, "results={results:?}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_returns_matching_lines_for_known_file_root() {
    let root = unique_temp_dir("grep-text-lines");
    std::fs::create_dir_all(&root).expect("create root");
    let file = root.join("sample.rs");
    std::fs::write(
        &file,
        "fn unrelated() {}\nif step_type == \"run_cmd\" {\n    normalize_run_cmd_call();\n}\n",
    )
    .expect("write sample file");

    let out = execute(json!({
        "action": "grep_text",
        "query": "run_cmd",
        "root": file.to_string_lossy().to_string(),
        "max_results": 10
    }))
    .expect("grep_text succeeds");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    assert_eq!(out.get("match_count").and_then(Value::as_u64), Some(2));
    let matches = out
        .get("matches")
        .and_then(Value::as_array)
        .expect("matches array");
    assert_eq!(matches[0].get("line").and_then(Value::as_u64), Some(2));
    assert_eq!(
        matches[0].get("start_byte").and_then(Value::as_u64),
        Some(35)
    );
    assert_eq!(matches[0]["matched_text"], "run_cmd");
    assert_eq!(matches[0]["range_handle"]["start_byte"], 35);
    assert!(matches[0]
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("step_type") && text.contains("run_cmd")));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_returns_bounded_context_and_multiline_provenance() {
    let root = unique_temp_dir("grep-context-multiline");
    std::fs::create_dir_all(&root).expect("create root");
    let file = root.join("sample.rs");
    std::fs::write(&file, "before\nlet value = 1;\nfinish(value);\nafter\n").expect("write sample");

    let out = execute(json!({
        "action": "grep_text",
        "query": "let value.*finish\\(value\\)",
        "pattern_kind": "regex",
        "root": file.to_string_lossy().to_string(),
        "multiline": true,
        "context_before": 1,
        "context_after": 1,
        "max_results": 10
    }))
    .expect("multiline grep");

    assert_eq!(out["schema_version"], 2);
    assert_eq!(out["multiline"], true);
    assert_eq!(out["total_match_count"], 1);
    let matched = &out["matches"][0];
    assert_eq!(matched["line"], 2);
    assert_eq!(matched["end_line"], 3);
    assert_eq!(matched["context_before"][0]["text"], "before");
    assert_eq!(matched["context_after"][0]["text"], "after");
    assert!(matched["end_byte"].as_u64() > matched["start_byte"].as_u64());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_enforces_file_and_total_byte_budgets() {
    let root = unique_temp_dir("grep-byte-budgets");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("a-large.txt"), "needle\npadding\n").expect("write large fixture");
    std::fs::write(root.join("b-next.txt"), "needle\n").expect("write next fixture");

    let out = execute(json!({
        "action": "grep_text",
        "query": "needle",
        "root": root.to_string_lossy().to_string(),
        "max_file_bytes": 8,
        "max_scan_bytes": 8,
        "max_results": 10
    }))
    .expect("bounded grep");

    assert_eq!(out["total_match_count"], 1);
    assert_eq!(out["skipped_large_files"], 1);
    assert_eq!(out["scanned_bytes"], 7);
    assert_eq!(out["page"]["scan_truncated"], true);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_can_match_case_insensitively() {
    let root = unique_temp_dir("grep-text-case-insensitive");
    std::fs::create_dir_all(&root).expect("create root");
    let file = root.join("release_checklist.md");
    std::fs::write(&file, "# Release Checklist\n").expect("write sample file");

    let out = execute(json!({
        "action": "grep_text",
        "query": "release",
        "path": file.to_string_lossy().to_string(),
        "case_insensitive": true,
        "max_results": 10
    }))
    .expect("grep_text succeeds");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    assert_eq!(out.get("match_count").and_then(Value::as_u64), Some(1));
    assert_eq!(
        out.get("case_insensitive").and_then(Value::as_bool),
        Some(true)
    );
    let matches = out
        .get("matches")
        .and_then(Value::as_array)
        .expect("matches array");
    assert!(matches[0]
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("Release Checklist")));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_sorts_by_mtime_with_path_tie_breaking() {
    let root = unique_temp_dir("find-name-mtime");
    std::fs::create_dir_all(&root).expect("create root");
    let older = root.join("match-a.txt");
    let newer = root.join("match-b.txt");
    std::fs::write(&older, "older\n").expect("write older");
    let old_time = std::fs::metadata(&older)
        .and_then(|metadata| metadata.modified())
        .expect("older mtime");
    for attempt in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&newer, format!("newer-{attempt}\n")).expect("write newer");
        let new_time = std::fs::metadata(&newer)
            .and_then(|metadata| metadata.modified())
            .expect("newer mtime");
        if new_time > old_time {
            break;
        }
    }

    let descending = execute(json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy().to_string(),
        "sort_by": "mtime_desc",
        "max_results": 10
    }))
    .expect("mtime descending");
    let ascending = execute(json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy().to_string(),
        "sort_by": "mtime_asc",
        "max_results": 10
    }))
    .expect("mtime ascending");

    assert!(descending["results"][0]
        .as_str()
        .is_some_and(|path| path.ends_with("match-b.txt")));
    assert!(ascending["results"][0]
        .as_str()
        .is_some_and(|path| path.ends_with("match-a.txt")));
    assert_eq!(descending["sort_by"], "mtime_desc");
    assert_eq!(ascending["sort_by"], "mtime_asc");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_accepts_ordered_wildcard_query() {
    let root = unique_temp_dir("grep-text-ordered-wildcard");
    std::fs::create_dir_all(&root).expect("create root");
    let file = root.join("sample.rs");
    std::fs::write(&file, "if step_type == \"run_cmd\" {\n}\n").expect("write sample file");

    let out = execute(json!({
        "action": "grep_text",
        "query": "type.*run_cmd",
        "pattern_kind": "regex",
        "path": file.to_string_lossy().to_string(),
        "max_results": 10
    }))
    .expect("grep_text succeeds with ordered wildcard query");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    assert_eq!(out.get("match_count").and_then(Value::as_u64), Some(1));
    let matches = out
        .get("matches")
        .and_then(Value::as_array)
        .expect("matches array");
    assert!(matches[0]
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("step_type") && text.contains("run_cmd")));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_accepts_path_alias_for_search_root() {
    let root = unique_temp_dir("grep-text-path-alias");
    std::fs::create_dir_all(&root).expect("create root");
    let file = root.join("sample.rs");
    std::fs::write(&file, "if step_type == \"run_cmd\" {}\n").expect("write sample file");

    let out = execute(json!({
        "action": "grep_text",
        "query": "run_cmd",
        "path": file.to_string_lossy().to_string(),
        "max_results": 10
    }))
    .expect("grep_text succeeds with path alias");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let root_value = out.get("root").and_then(Value::as_str).unwrap_or_default();
    assert!(
        root_value.ends_with("sample.rs"),
        "root should reflect the path alias target, got {root_value:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_filters_by_file_pattern() {
    let root = unique_temp_dir("grep-text-file-pattern");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("prompt_utils.rs"),
        "if step_type == \"run_cmd\" {}\n",
    )
    .expect("write target");
    std::fs::write(root.join("other.rs"), "if step_type == \"run_cmd\" {}\n")
        .expect("write sibling");

    let out = execute(json!({
        "action": "grep_text",
        "query": "run_cmd",
        "pattern": "prompt_utils.rs",
        "root": root.to_string_lossy().to_string(),
        "max_results": 10
    }))
    .expect("grep_text succeeds with file pattern");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert!(results[0].ends_with("prompt_utils.rs"));
    assert!(out
        .get("patterns")
        .and_then(Value::as_array)
        .is_some_and(|patterns| patterns
            .iter()
            .any(|item| item.as_str() == Some("prompt_utils.rs"))));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_does_not_mix_filename_matches_into_content_results() {
    let root = unique_temp_dir("grep-text-name-fallback");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("my_abcd.txt"), "content without target\n").expect("write target");
    std::fs::write(root.join("other.txt"), "content without target\n").expect("write other");

    let out = execute(json!({
        "action": "grep_text",
        "query": "abcd",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 1,
        "max_results": 10
    }))
    .expect("grep_text succeeds");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(0));
    assert_eq!(out.get("match_count").and_then(Value::as_u64), Some(0));
    assert_eq!(
        out.get("name_fallback_used").and_then(Value::as_bool),
        Some(false)
    );
    assert!(out.get("name_results").is_none());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_pattern_with_extension_filters_extension() {
    let root = unique_temp_dir("find-name-ext-pattern");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("execution_intent_routing_repair_plan_20260509.md"),
        "",
    )
    .expect("write md target");
    std::fs::write(root.join("execution_intent_route_trace_cases.txt"), "")
        .expect("write txt sibling");

    let out = execute(json!({
        "action": "find_name",
        "pattern": "*execution_intent*.md",
        "target_kind": "file",
        "root": root.to_string_lossy().to_string(),
        "max_depth": 1,
        "max_results": 10
    }))
    .expect("find_name succeeds with extension pattern");

    assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
    let results = out
        .get("results")
        .and_then(Value::as_array)
        .expect("results array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert!(results[0].ends_with("execution_intent_routing_repair_plan_20260509.md"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_returns_stable_cursor_pages_and_snapshot_hash() {
    let root = unique_temp_dir("find-name-pages");
    std::fs::create_dir_all(&root).expect("create root");
    for name in ["match-c.txt", "match-a.txt", "match-d.txt", "match-b.txt"] {
        std::fs::write(root.join(name), "fixture\n").expect("write fixture");
    }
    let args = json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy().to_string(),
        "max_results": 2
    });

    let first = execute(args.clone()).expect("first page");
    let cursor = first["page"]["next_cursor"]
        .as_str()
        .expect("opaque next cursor")
        .to_string();
    let mut second_args = args;
    second_args["cursor"] = json!(cursor);
    let second = execute(second_args).expect("second page");

    assert_eq!(first["total_count"], 4);
    assert_eq!(first["returned_count"], 2);
    assert_eq!(first["page"]["cursor"], 0);
    assert_eq!(first["page"]["has_more"], true);
    assert_eq!(second["returned_count"], 2);
    assert_eq!(second["page"]["cursor"], 2);
    assert!(second["page"]["previous_cursor"].is_string());
    assert_eq!(first["page"]["legacy_next_offset"], 2);
    assert_eq!(second["page"]["has_more"], false);
    assert_eq!(first["snapshot_sha256"], second["snapshot_sha256"]);
    assert_ne!(first["results"], second["results"]);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_rejects_cursor_from_a_different_query() {
    let root = unique_temp_dir("cursor-query");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("alpha-one.txt"), "").expect("write alpha one");
    std::fs::write(root.join("alpha-two.txt"), "").expect("write alpha two");
    std::fs::write(root.join("beta.txt"), "").expect("write beta");
    let first = execute(json!({
        "action": "find_name",
        "pattern": "alpha",
        "root": root.to_string_lossy(),
        "max_results": 1
    }))
    .expect("first query");
    let error = execute(json!({
        "action": "find_name",
        "pattern": "beta",
        "root": root.to_string_lossy(),
        "max_results": 1,
        "cursor": first["page"]["next_cursor"].clone()
    }))
    .expect_err("cursor must be query bound");
    assert_eq!(error, "cursor_query_mismatch");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn opaque_cursor_ignores_changing_runtime_memory_context() {
    let root = unique_temp_dir("cursor-runtime-memory");
    std::fs::create_dir_all(&root).expect("create root");
    for name in ["match-a.txt", "match-b.txt", "match-c.txt"] {
        std::fs::write(root.join(name), "fixture\n").expect("write fixture");
    }
    let mut args = json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy(),
        "max_results": 1,
        "_memory": {"context": "first request"}
    });

    let first = execute(args.clone()).expect("first page");
    args["cursor"] = first["page"]["next_cursor"].clone();
    args["_memory"] = json!({"context": "changed between tasks"});
    let second = execute(args).expect("runtime memory must not change query identity");

    assert_eq!(
        first["page"]["query_sha256"],
        second["page"]["query_sha256"]
    );
    assert_eq!(second["page"]["cursor"], 1);
    assert_ne!(first["results"], second["results"]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn empty_optional_cursor_starts_from_the_first_page() {
    let root = unique_temp_dir("empty-cursor");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("match-a.txt"), "fixture\n").expect("write fixture");

    let out = execute(json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy(),
        "cursor": "",
        "max_results": 1
    }))
    .expect("empty optional cursor behaves like omission");

    assert_eq!(out["page"]["cursor"], 0);
    assert_eq!(out["returned_count"], 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_marks_cursor_stale_when_snapshot_changes() {
    let root = unique_temp_dir("cursor-stale");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("match-a.txt"), "").expect("write a");
    std::fs::write(root.join("match-b.txt"), "").expect("write b");
    let args = json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy(),
        "max_results": 1
    });
    let first = execute(args.clone()).expect("first page");
    std::fs::write(root.join("match-c.txt"), "").expect("change snapshot");
    let mut next_args = args;
    next_args["cursor"] = first["page"]["next_cursor"].clone();
    let stale = execute(next_args).expect("stale snapshot is structured");
    assert_eq!(stale["completeness"], "stale_snapshot");
    assert_eq!(stale["results"], json!([]));
    assert_eq!(stale["continuation"]["kind"], "new_snapshot");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_default_search_crosses_old_depth_boundary() {
    let root = unique_temp_dir("default-deep");
    let mut nested = root.clone();
    for index in 0..12 {
        nested.push(format!("level-{index}"));
    }
    std::fs::create_dir_all(&nested).expect("create deep root");
    std::fs::write(nested.join("deep-target.txt"), "").expect("write deep target");
    let out = execute(json!({
        "action": "find_name",
        "pattern": "deep-target.txt",
        "exact": true,
        "target_kind": "file",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("unbounded-depth default search");
    assert_eq!(out["returned_count"], 1);
    assert_eq!(out["completeness"], "complete");
    assert_eq!(out["effective_policy"]["max_depth"], Value::Null);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_default_scan_exceeds_old_eight_hundred_entry_budget() {
    let root = unique_temp_dir("default-large");
    std::fs::create_dir_all(&root).expect("create root");
    for index in 0..805 {
        std::fs::write(root.join(format!("noise-{index:04}.txt")), "").expect("write noise");
    }
    std::fs::write(root.join("wanted-after-800.txt"), "").expect("write target");
    let out = execute(json!({
        "action": "find_name",
        "pattern": "wanted-after-800.txt",
        "exact": true,
        "target_kind": "file",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("large default search");
    assert_eq!(out["returned_count"], 1);
    assert_eq!(out["completeness"], "complete");
    assert!(out["scan"]["visited_files"].as_u64().unwrap_or_default() > 800);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn partial_zero_result_is_not_reported_as_complete_absence() {
    let root = unique_temp_dir("partial-zero");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("a.txt"), "").expect("write a");
    std::fs::write(root.join("z-target.txt"), "").expect("write target");
    let out = execute(json!({
        "action": "find_name",
        "pattern": "z-target.txt",
        "exact": true,
        "target_kind": "file",
        "root": root.to_string_lossy(),
        "__test_hard_entry_limit": 1,
        "max_results": 10
    }))
    .expect("internally bounded search");
    assert_eq!(out["returned_count"], 0);
    assert_eq!(out["completeness"], "partial_hard_limit");
    assert_eq!(out["total_count_is_complete"], false);
    assert_eq!(out["continuation"]["kind"], "scan_frontier");
    let mut continuation = out["continuation"]["token"]
        .as_str()
        .expect("frontier token")
        .to_string();
    let mut found = false;
    for _ in 0..4 {
        let page = execute(json!({
            "action": "find_name",
            "pattern": "z-target.txt",
            "exact": true,
            "target_kind": "file",
            "root": root.to_string_lossy(),
            "__test_hard_entry_limit": 1,
            "max_results": 10,
            "scan_continuation": continuation,
        }))
        .expect("resume traversal frontier");
        if page["returned_count"].as_u64().unwrap_or_default() > 0 {
            found = true;
            break;
        }
        let Some(next) = page["continuation"]["token"].as_str() else {
            break;
        };
        continuation = next.to_string();
    }
    assert!(
        found,
        "target must be reachable through traversal continuation"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_images_returns_stable_pages_and_bounded_metadata() {
    let root = unique_temp_dir("find-images-pages");
    std::fs::create_dir_all(root.join("a")).expect("create a");
    std::fs::create_dir_all(root.join("b")).expect("create b");
    let mut png = vec![0; 24];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[16..20].copy_from_slice(&320u32.to_be_bytes());
    png[20..24].copy_from_slice(&200u32.to_be_bytes());
    std::fs::write(root.join("a/one.png"), &png).expect("write one");
    std::fs::write(root.join("a/two.png"), &png).expect("write two");
    std::fs::write(root.join("b/three.png"), &png).expect("write three");
    std::fs::write(root.join("ignored.txt"), "not an image").expect("write ignored");

    let args = json!({
        "action": "find_images",
        "root": root.to_string_lossy().to_string(),
        "max_results": 2,
        "max_dirs": 1
    });
    let first = execute(args.clone()).expect("first image page");
    let mut second_args = args;
    second_args["cursor"] = first["page"]["next_cursor"].clone();
    let second = execute(second_args).expect("second image page");

    assert_eq!(first["schema_version"], 2);
    assert_eq!(first["count"], 2);
    assert_eq!(first["total_count"], 3);
    assert_eq!(first["images"][0]["mime_type"], "image/png");
    assert_eq!(first["images"][0]["width"], 320);
    assert_eq!(first["images"][0]["height"], 200);
    assert_eq!(first["directories_by_count"][0]["count"], 2);
    assert_eq!(first["directories_truncated"], true);
    assert_eq!(first["page"]["has_more"], true);
    assert_eq!(second["count"], 1);
    assert_eq!(first["snapshot_sha256"], second["snapshot_sha256"]);
    assert_ne!(first["results"], second["results"]);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_pages_matching_lines_without_losing_total_counts() {
    let root = unique_temp_dir("grep-pages");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("events.log"),
        "hit one\nskip\nhit two\nhit three\n",
    )
    .expect("write fixture");
    let args = json!({
        "action": "grep_text",
        "query": "hit",
        "root": root.to_string_lossy().to_string(),
        "max_results": 2
    });

    let first = execute(args.clone()).expect("first page");
    let mut second_args = args;
    second_args["cursor"] = first["page"]["next_cursor"].clone();
    let second = execute(second_args).expect("second page");

    assert_eq!(first["match_count"], 2);
    assert_eq!(first["total_match_count"], 3);
    assert_eq!(first["page"]["has_more"], true);
    assert_eq!(second["match_count"], 1);
    assert_eq!(second["total_match_count"], 3);
    assert_eq!(second["matches"][0]["line"], 4);
    assert_eq!(first["snapshot_sha256"], second["snapshot_sha256"]);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resolve_path_rejects_existing_paths_outside_workspace() {
    let parent = unique_temp_dir("workspace-fence");
    let workspace = parent.join("workspace");
    let outside = parent.join("outside");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&outside).expect("create outside");

    let error = resolve_path(&workspace, outside.to_string_lossy().as_ref(), false)
        .expect_err("outside path must be rejected");

    assert!(error.contains("outside workspace"), "error={error:?}");
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn resolve_path_allows_admin_service_account_scope_outside_workspace() {
    let parent = unique_temp_dir("admin-workspace-fence");
    let workspace = parent.join("workspace");
    let outside = parent.join("outside");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&outside).expect("create outside");

    let resolved = resolve_path(&workspace, outside.to_string_lossy().as_ref(), true)
        .expect("admin outside path must resolve");

    assert_eq!(resolved, outside.canonicalize().expect("canonical outside"));
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn admin_system_root_normalizes_to_the_host_root() {
    let workspace = unique_temp_dir("admin-system-root");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let resolved = resolve_path(&workspace, std::path::MAIN_SEPARATOR_STR, true)
        .expect("admin host root must resolve");

    assert_eq!(
        resolved,
        std::path::Path::new(std::path::MAIN_SEPARATOR_STR)
            .canonicalize()
            .expect("canonical host root")
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn runner_permission_context_controls_admin_external_root_access() {
    assert!(!context_allows_path_outside_workspace(None));
    assert!(!context_allows_path_outside_workspace(Some(&json!({
        "permissions": {"allow_path_outside_workspace": false}
    }))));
    assert!(context_allows_path_outside_workspace(Some(&json!({
        "permissions": {"allow_path_outside_workspace": true}
    }))));
}

#[cfg(unix)]
#[test]
fn grep_text_does_not_follow_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let root = unique_temp_dir("symlink-fence");
    let outside = unique_temp_dir("symlink-outside");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&outside).expect("create outside");
    std::fs::write(outside.join("secret.txt"), "needle\n").expect("write outside fixture");
    symlink(&outside, root.join("linked")).expect("create directory symlink");

    let out = execute(json!({
        "action": "grep_text",
        "query": "needle",
        "root": root.to_string_lossy().to_string(),
        "max_results": 10
    }))
    .expect("grep succeeds");

    assert_eq!(out["total_match_count"], 0);
    assert_eq!(out["matches"], json!([]));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
}

fn cache_context(database_path: &std::path::Path) -> Value {
    json!({
        "skill_storage": {
            "storage_kind": "sqlite",
            "skill_name": "fs_search",
            "schema_version": 1,
            "database_path": database_path.to_string_lossy()
        }
    })
}

#[test]
fn find_name_supports_path_globs_and_smart_case() {
    let root = unique_temp_dir("typed-glob-case");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("one/src")).expect("create source dir");
    std::fs::create_dir_all(root.join("one/tests")).expect("create tests dir");
    std::fs::write(root.join("one/src/Widget.RS"), "").expect("write smart-case target");
    std::fs::write(root.join("one/tests/widget.rs"), "").expect("write excluded target");

    let smart = execute(json!({
        "action": "find_name",
        "glob": "**/src/*.rs",
        "case_mode": "smart",
        "target_kind": "file",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("smart-case glob search");
    assert_eq!(smart["results"].as_array().map(Vec::len), Some(1));
    assert!(smart["results"][0]
        .as_str()
        .is_some_and(|path| path.ends_with("one/src/Widget.RS")));

    let sensitive = execute(json!({
        "action": "find_name",
        "glob": "**/src/*.rs",
        "case_mode": "sensitive",
        "target_kind": "file",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("case-sensitive glob search");
    assert_eq!(sensitive["returned_count"], 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_supports_prefix_and_suffix_modes() {
    let root = unique_temp_dir("typed-name-modes");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create root");
    for name in ["alpha-report.md", "beta-report.md", "alpha-notes.txt"] {
        std::fs::write(root.join(name), "").expect("write fixture");
    }

    let prefix = execute(json!({
        "action": "find_name",
        "pattern": "alpha-",
        "match_mode": "prefix",
        "target_kind": "file",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("prefix search");
    assert_eq!(prefix["returned_count"], 2);

    let suffix = execute(json!({
        "action": "find_name",
        "pattern": "-report.md",
        "match_mode": "suffix",
        "target_kind": "file",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("suffix search");
    assert_eq!(suffix["returned_count"], 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_name_fuzzy_mode_tolerates_typos_and_ranks_relevance() {
    let root = unique_temp_dir("fuzzy-ranking");
    std::fs::create_dir_all(root.join("agent-runtime-cache")).expect("create fuzzy directory");
    std::fs::write(root.join("agent-runtime.toml"), "fixture\n").expect("write close match");
    std::fs::write(root.join("agent-runtime-backup.toml"), "fixture\n")
        .expect("write broader match");
    std::fs::write(root.join("unrelated.toml"), "fixture\n").expect("write unrelated");

    let files = execute(json!({
        "action": "find_name",
        "pattern": "agent-runtmie.toml",
        "match_mode": "fuzzy",
        "target_kind": "file",
        "root": root.to_string_lossy().to_string(),
        "max_results": 10
    }))
    .expect("fuzzy file search succeeds");

    assert_eq!(files["match_mode"], "fuzzy");
    assert_eq!(files["sort_by"], "relevance");
    assert_eq!(
        files["results"][0].as_str(),
        Some(root.join("agent-runtime.toml").to_string_lossy().as_ref())
    );
    assert!(files["results"]
        .as_array()
        .is_some_and(|items| items.iter().all(|item| !item
            .as_str()
            .unwrap_or_default()
            .ends_with("unrelated.toml"))));

    let directories = execute(json!({
        "action": "find_name",
        "pattern": "agent-runtmie-cache",
        "match_mode": "fuzzy",
        "target_kind": "dir",
        "root": root.to_string_lossy().to_string(),
        "max_results": 10
    }))
    .expect("fuzzy directory search succeeds");
    assert_eq!(directories["count"], 1);
    assert!(directories["results"][0]
        .as_str()
        .is_some_and(|path| path.ends_with("agent-runtime-cache")));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grep_text_distinguishes_literal_regex_and_output_modes() {
    let root = unique_temp_dir("typed-grep-modes");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("create source dir");
    std::fs::write(root.join("src/main.rs"), "fn   main() {}\n").expect("write source");
    std::fs::write(root.join("src/main.txt"), "fn   main() {}\n").expect("write non-rust");

    let literal = execute(json!({
        "action": "grep_text",
        "query": "fn\\s+main",
        "pattern_kind": "literal",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("literal search");
    assert_eq!(literal["total_match_count"], 0);

    let paths = execute(json!({
        "action": "grep_text",
        "query": "fn\\s+main",
        "pattern_kind": "regex",
        "output_mode": "paths",
        "globs": ["**/*.rs"],
        "case_mode": "sensitive",
        "root": root.to_string_lossy(),
        "max_results": 10
    }))
    .expect("regex paths search");
    assert_eq!(paths["results"].as_array().map(Vec::len), Some(1));
    assert_eq!(paths["matches"], json!([]));
    assert!(paths["results"][0]
        .as_str()
        .is_some_and(|path| path.ends_with("src/main.rs")));

    let count = execute(json!({
        "action": "grep_text",
        "query": "fn\\s+main",
        "pattern_kind": "regex",
        "output_mode": "count",
        "globs": ["**/*.rs"],
        "root": root.to_string_lossy(),
        "max_results": 1
    }))
    .expect("regex count search");
    assert_eq!(count["count"], 1);
    assert_eq!(count["results"], json!([]));
    assert_eq!(count["matches"], json!([]));
    assert_eq!(count["has_more"], false);
    assert_eq!(count["continuation"], Value::Null);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn opaque_continuation_reuses_declared_skill_snapshot_cache() {
    let root = unique_temp_dir("snapshot-cache-root");
    let storage = unique_temp_dir("snapshot-cache-storage");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&storage);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&storage).expect("create storage");
    for name in ["match-a.txt", "match-b.txt", "match-c.txt"] {
        std::fs::write(root.join(name), "fixture\n").expect("write fixture");
    }
    let context = cache_context(&storage.join("fs-search.sqlite3"));
    let args = json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy(),
        "max_results": 1,
        "__test_backend": "rust"
    });

    let first = execute_with_context(args.clone(), Some(&context)).expect("first cached page");
    assert_eq!(first["cache_reused"], false);
    assert_eq!(first["cache_status"], "stored");
    let mut second_args = args;
    second_args["cursor"] = first["page"]["next_cursor"].clone();
    let second = execute_with_context(second_args, Some(&context)).expect("cached second page");
    assert_eq!(second["cache_reused"], true);
    assert_eq!(second["scan"]["cache_reused"], true);
    assert_eq!(second["page"]["cursor"], 1);
    assert_ne!(first["results"], second["results"]);
    assert_eq!(first["snapshot_sha256"], second["snapshot_sha256"]);

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(storage);
}

#[test]
fn snapshot_cache_returns_stale_status_after_tree_change() {
    let root = unique_temp_dir("snapshot-cache-stale-root");
    let storage = unique_temp_dir("snapshot-cache-stale-storage");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&storage);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&storage).expect("create storage");
    std::fs::write(root.join("match-a.txt"), "a\n").expect("write a");
    std::fs::write(root.join("match-b.txt"), "b\n").expect("write b");
    let context = cache_context(&storage.join("fs-search.sqlite3"));
    let args = json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy(),
        "max_results": 1,
        "__test_backend": "rust"
    });
    let first = execute_with_context(args.clone(), Some(&context)).expect("first page");
    std::fs::write(root.join("match-c.txt"), "c\n").expect("mutate tree");
    let mut next_args = args;
    next_args["cursor"] = first["page"]["next_cursor"].clone();
    let stale = execute_with_context(next_args, Some(&context)).expect("stale cache response");
    assert_eq!(stale["cache_reused"], true);
    assert_eq!(stale["completeness"], "stale_snapshot");
    assert_eq!(stale["results"], json!([]));
    assert_eq!(stale["continuation"]["kind"], "new_snapshot");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(storage);
}

#[test]
fn snapshot_cache_invalidates_when_ignore_policy_file_changes() {
    let root = unique_temp_dir("snapshot-cache-ignore-root");
    let storage = unique_temp_dir("snapshot-cache-ignore-storage");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&storage);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&storage).expect("create storage");
    std::fs::write(root.join(".gitignore"), "ignored.txt\n").expect("write ignore file");
    for name in ["match-a.txt", "match-b.txt", "ignored.txt"] {
        std::fs::write(root.join(name), "fixture\n").expect("write fixture");
    }
    let context = cache_context(&storage.join("fs-search.sqlite3"));
    let args = json!({
        "action": "find_ext",
        "ext": "txt",
        "root": root.to_string_lossy(),
        "max_results": 1,
        "__test_backend": "rust"
    });
    let first = execute_with_context(args.clone(), Some(&context)).expect("first page");
    assert_eq!(first["known_match_count"], 2);
    std::fs::write(root.join(".gitignore"), "# no ignored entries now\n")
        .expect("change ignore file");
    let mut next_args = args;
    next_args["cursor"] = first["page"]["next_cursor"].clone();
    let stale = execute_with_context(next_args, Some(&context)).expect("stale ignore response");
    assert_eq!(stale["completeness"], "stale_snapshot");
    assert_eq!(stale["continuation"]["reason_code"], "stale_snapshot");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(storage);
}

#[test]
fn expired_snapshot_requires_a_new_snapshot_without_rescanning() {
    let root = unique_temp_dir("snapshot-cache-expired-root");
    let storage = unique_temp_dir("snapshot-cache-expired-storage");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&storage);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&storage).expect("create storage");
    for name in ["match-a.txt", "match-b.txt"] {
        std::fs::write(root.join(name), "fixture\n").expect("write fixture");
    }
    let database = storage.join("fs-search.sqlite3");
    let context = cache_context(&database);
    let args = json!({
        "action": "find_name",
        "pattern": "match-",
        "root": root.to_string_lossy(),
        "max_results": 1,
        "__test_backend": "rust"
    });
    let first = execute_with_context(args.clone(), Some(&context)).expect("first page");
    rusqlite::Connection::open(&database)
        .expect("open cache")
        .execute(
            "UPDATE fs_search_snapshots SET created_at = created_at - 120",
            [],
        )
        .expect("expire cache row");
    let mut next_args = args;
    next_args["cursor"] = first["page"]["next_cursor"].clone();
    let expired = execute_with_context(next_args, Some(&context)).expect("expired response");
    assert_eq!(expired["completeness"], "stale_snapshot");
    assert_eq!(expired["cache_status"], "miss_or_expired");
    assert_eq!(
        expired["continuation"]["reason_code"],
        "snapshot_cache_miss"
    );
    assert_eq!(expired["scan"]["visited_entries"], Value::Null);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(storage);
}

#[test]
fn snapshot_cache_evicts_oldest_entry_at_the_bounded_capacity() {
    let root = unique_temp_dir("snapshot-cache-eviction-root");
    let storage = unique_temp_dir("snapshot-cache-eviction-storage");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&storage);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&storage).expect("create storage");
    for name in ["common-a.txt", "common-b.txt"] {
        std::fs::write(root.join(name), "fixture\n").expect("write fixture");
    }
    let database = storage.join("fs-search.sqlite3");
    let context = cache_context(&database);
    let first_args = json!({
        "action": "find_name",
        "pattern": "common-",
        "root": root.to_string_lossy(),
        "max_results": 1,
        "__test_backend": "rust"
    });
    let first = execute_with_context(first_args.clone(), Some(&context)).expect("first page");
    let db = rusqlite::Connection::open(&database).expect("open cache");
    db.execute("UPDATE fs_search_snapshots SET last_accessed_at = 0", [])
        .expect("age first row");
    drop(db);
    for index in 0..16 {
        execute_with_context(
            json!({
                "action": "find_name",
                "pattern": format!("distinct-{index}"),
                "root": root.to_string_lossy(),
                "max_results": 1,
                "__test_backend": "rust"
            }),
            Some(&context),
        )
        .expect("store distinct snapshot");
    }
    let db = rusqlite::Connection::open(&database).expect("reopen cache");
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM fs_search_snapshots", [], |row| {
            row.get(0)
        })
        .expect("count cache rows");
    assert_eq!(count, 16);
    drop(db);
    let mut next_args = first_args;
    next_args["cursor"] = first["page"]["next_cursor"].clone();
    let evicted = execute_with_context(next_args, Some(&context)).expect("evicted response");
    assert_eq!(evicted["completeness"], "stale_snapshot");
    assert_eq!(evicted["cache_status"], "miss_or_expired");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(storage);
}
