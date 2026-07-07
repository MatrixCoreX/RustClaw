use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.fs_search.execution_failed");
    assert_eq!(extra["retryable"], false);
}

fn unique_temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rustclaw-fs-search-{name}-{}", std::process::id()))
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
fn find_name_directory_target_ignores_unrelated_file_budget() {
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
        "max_files": 4,
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
fn find_name_checks_shallow_files_before_deep_scan_budget() {
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
        "max_files": 1,
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
    assert!(matches[0]
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("step_type") && text.contains("run_cmd")));

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
fn grep_text_accepts_ordered_wildcard_query() {
    let root = unique_temp_dir("grep-text-ordered-wildcard");
    std::fs::create_dir_all(&root).expect("create root");
    let file = root.join("sample.rs");
    std::fs::write(&file, "if step_type == \"run_cmd\" {\n}\n").expect("write sample file");

    let out = execute(json!({
        "action": "grep_text",
        "query": "type.*run_cmd",
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
fn grep_text_surfaces_name_matches_when_content_has_no_hits() {
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
    assert_eq!(out.get("name_count").and_then(Value::as_u64), Some(1));
    let name_results = out
        .get("name_results")
        .and_then(Value::as_array)
        .expect("name_results array");
    assert!(name_results
        .iter()
        .any(|v| v.as_str().is_some_and(|path| path.ends_with("my_abcd.txt"))));

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
