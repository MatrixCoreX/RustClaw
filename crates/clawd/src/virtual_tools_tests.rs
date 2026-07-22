use super::{
    canonicalize_legacy_tool_call, normalize_virtual_tool_arg_aliases, rewrite_virtual_tool_call,
};
use serde_json::json;

#[test]
fn legacy_system_basic_path_batch_facts_canonicalizes_to_fs_basic() {
    let canonical = canonicalize_legacy_tool_call(
        "system_basic",
        json!({"action":"path_batch_facts", "paths":["README.md"]}),
    )
    .expect("canonical");
    assert_eq!(canonical.tool, "fs_basic");
    assert_eq!(
        canonical.args.get("action").and_then(|v| v.as_str()),
        Some("stat_paths")
    );
}

#[test]
fn legacy_system_basic_count_inventory_canonicalizes_to_fs_basic_count_entries() {
    let canonical = canonicalize_legacy_tool_call(
        "system_basic",
        json!({"action":"count_inventory", "path":"scripts"}),
    )
    .expect("canonical");
    assert_eq!(canonical.tool, "fs_basic");
    assert_eq!(
        canonical.args.get("action").and_then(|v| v.as_str()),
        Some("count_entries")
    );
}

#[test]
fn legacy_system_basic_extract_field_canonicalizes_to_config_basic() {
    let canonical = canonicalize_legacy_tool_call(
        "system_basic",
        json!({"action":"extract_field", "path":"Cargo.toml", "field_path":"workspace.package.version"}),
    )
    .expect("canonical");
    assert_eq!(canonical.tool, "config_basic");
    assert_eq!(
        canonical.args.get("action").and_then(|v| v.as_str()),
        Some("read_field")
    );
}

#[test]
fn legacy_system_basic_validate_structured_canonicalizes_to_config_basic_validate() {
    let canonical = canonicalize_legacy_tool_call(
        "system_basic",
        json!({"action":"validate_structured", "path":"configs/config.toml", "format":"toml"}),
    )
    .expect("canonical");
    assert_eq!(canonical.tool, "config_basic");
    assert_eq!(
        canonical.args.get("action").and_then(|v| v.as_str()),
        Some("validate")
    );
}

#[test]
fn legacy_fs_search_find_ext_canonicalizes_to_fs_basic_find_entries() {
    let canonical = canonicalize_legacy_tool_call(
        "fs_search",
        json!({"action":"find_ext", "root":"scripts", "ext":"sh"}),
    )
    .expect("canonical");
    assert_eq!(canonical.tool, "fs_basic");
    assert_eq!(
        canonical.args.get("action").and_then(|v| v.as_str()),
        Some("find_entries")
    );
    assert_eq!(
        canonical.args.get("ext").and_then(|v| v.as_str()),
        Some("sh")
    );
}

#[test]
fn legacy_fs_search_grep_text_drops_query_as_filename_filter() {
    let canonical = canonicalize_legacy_tool_call(
        "fs_search",
        json!({
            "action": "grep_text",
            "root": ".",
            "query": "FirstLayerDecision",
            "pattern": "FirstLayerDecision",
            "patterns": ["FirstLayerDecision", "*.rs"]
        }),
    )
    .expect("canonical");

    assert_eq!(canonical.tool, "fs_basic");
    assert_eq!(
        canonical.args.get("action").and_then(|v| v.as_str()),
        Some("grep_text")
    );
    assert!(canonical.args.get("pattern").is_none());
    assert_eq!(canonical.args.get("patterns"), Some(&json!(["*.rs"])));
}

#[test]
fn fs_basic_grep_text_pattern_alias_normalizes_to_required_query() {
    let mut args = json!({
        "action": "grep_text",
        "path": "docs/release_checklist.md",
        "pattern": "release"
    });

    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("grep_text")
    );
    assert_eq!(args.get("query").and_then(|v| v.as_str()), Some("release"));
    assert!(args.get("pattern").is_none());
}

#[test]
fn fs_basic_read_text_range_offset_limit_normalizes_to_line_bounds() {
    let mut args = json!({
        "action": "read_text_range",
        "path": "README.md",
        "offset": 1,
        "limit": 5
    });

    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("read_text_range")
    );
    assert_eq!(args.get("start_line").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(args.get("end_line").and_then(|v| v.as_u64()), Some(5));
    assert_eq!(args.get("mode").and_then(|v| v.as_str()), Some("range"));
    assert!(args.get("limit").is_none());
    assert!(args.get("max_entries").is_none());

    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("read_range")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("start_line")
            .and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("end_line")
            .and_then(|v| v.as_u64()),
        Some(5)
    );
}

#[test]
fn fs_basic_read_text_range_limit_only_normalizes_to_n_not_max_entries() {
    let mut args = json!({
        "action": "read_text_range",
        "path": "README.md",
        "limit": 5
    });

    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    assert_eq!(args.get("n").and_then(|v| v.as_u64()), Some(5));
    assert!(args.get("limit").is_none());
    assert!(args.get("max_entries").is_none());
}

#[test]
fn fs_basic_read_text_range_field_selector_alias_normalizes_and_rewrites() {
    let mut args = json!({
        "action": "read_text_range",
        "path": "README.md",
        "selector": "title"
    });

    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    assert_eq!(
        args.get("field_selector").and_then(|v| v.as_str()),
        Some("title")
    );
    assert!(args.get("selector").is_none());

    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite
            .runtime_args
            .get("field_selector")
            .and_then(|v| v.as_str()),
        Some("title")
    );
}

#[test]
fn fs_basic_stat_paths_rewrites_to_system_basic_path_batch_facts() {
    let mut args = json!({"action":"stat", "path":"README.md"});
    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("path_batch_facts")
    );
    assert_eq!(
        rewrite.runtime_args.get("paths").and_then(|v| v.as_str()),
        Some("README.md")
    );
}

#[test]
fn fs_basic_count_entries_rewrites_to_system_basic_count_inventory() {
    let mut args = json!({"action":"count", "directory":"scripts"});
    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("count_inventory")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("scripts")
    );
}

#[test]
fn fs_basic_count_entries_filter_kind_directories_rewrites_to_dir_filter() {
    let mut args = json!({
        "action": "count_entries",
        "path": "scripts/nl_tests/fixtures/device_local",
        "filter_kind": "directories"
    });

    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    assert_eq!(
        args.get("kind_filter").and_then(|v| v.as_str()),
        Some("dir")
    );
    assert_eq!(args.get("count_dirs").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        args.get("count_files").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(args.get("dirs_only").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        args.get("files_only").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert!(args.get("filter_kind").is_none());

    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("count_inventory")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("kind_filter")
            .and_then(|v| v.as_str()),
        Some("dir")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("count_dirs")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("count_files")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn fs_basic_count_entries_extensions_alias_rewrites_to_ext_filter() {
    let mut args = json!({
        "action": "count_entries",
        "path": "scripts/nl_tests/fixtures/device_local",
        "extensions": ["md"]
    });

    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    let ext_filter = args
        .get("ext_filter")
        .and_then(|v| v.as_array())
        .expect("ext_filter array");
    assert_eq!(ext_filter, &vec![json!("md")]);
    assert!(args.get("extensions").is_none());

    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    let ext_filter = rewrite
        .runtime_args
        .get("ext_filter")
        .and_then(|v| v.as_array())
        .expect("runtime ext_filter array");
    assert_eq!(ext_filter, &vec![json!("md")]);
}

#[test]
fn fs_basic_find_entries_by_extension_rewrites_to_fs_search_find_ext() {
    let args = json!({"action":"find_entries", "root":"scripts", "extension":"sh"});
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_ext")
    );
    assert_eq!(
        rewrite.runtime_args.get("ext").and_then(|v| v.as_str()),
        Some("sh")
    );
}

#[test]
fn fs_basic_find_path_alias_rewrites_to_find_entries() {
    let mut args = json!({
        "action": "find_path",
        "root": "docs",
        "target_kind": "file",
        "max_results": 4
    });
    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("inventory_dir")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("docs")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("max_entries")
            .and_then(|v| v.as_u64()),
        Some(4)
    );
    assert!(rewrite.runtime_args.get("max_results").is_none());
    assert_eq!(
        rewrite
            .runtime_args
            .get("files_only")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn fs_basic_find_entries_ext_filter_alias_rewrites_to_find_ext() {
    let args = json!({"action":"find_entries", "root":"scripts", "ext_filter":"sh"});
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_ext")
    );
    assert_eq!(
        rewrite.runtime_args.get("ext").and_then(|v| v.as_str()),
        Some("sh")
    );
}

#[test]
fn fs_basic_find_entries_extension_names_alias_rewrites_to_find_ext() {
    let args = json!({
        "action": "find_entries",
        "target": "scripts/nl_tests/fixtures/device_local",
        "target_kind": "file",
        "names": [".db", ".sqlite", ".sqlite3", ".db3"]
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_ext")
    );
    assert_eq!(
        rewrite.runtime_args.get("root").and_then(|v| v.as_str()),
        Some("scripts/nl_tests/fixtures/device_local")
    );
    assert_eq!(
        rewrite.runtime_args.get("ext"),
        Some(&json!(["db", "sqlite", "sqlite3", "db3"]))
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("target_kind")
            .and_then(|v| v.as_str()),
        Some("file")
    );
}

#[test]
fn fs_basic_find_entries_directory_target_and_filter_rewrites_to_find_ext() {
    let args = json!({
        "action": "find_entries",
        "target": "scripts/nl_tests/fixtures/device_local",
        "target_kind": "file",
        "filter": "*.toml"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_ext")
    );
    assert_eq!(
        rewrite.runtime_args.get("root").and_then(|v| v.as_str()),
        Some("scripts/nl_tests/fixtures/device_local")
    );
    assert_eq!(
        rewrite.runtime_args.get("ext").and_then(|v| v.as_str()),
        Some("toml")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("target_kind")
            .and_then(|v| v.as_str()),
        Some("file")
    );
    assert!(rewrite.runtime_args.get("pattern").is_none());
}

#[test]
fn fs_basic_find_entries_name_pattern_alias_rewrites_to_name_search() {
    let args = json!({
        "action": "find_entries",
        "path": ".",
        "name_pattern": "*log*.md",
        "files_only": true,
        "recursive": true
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_name")
    );
    assert_eq!(
        rewrite.runtime_args.get("pattern").and_then(|v| v.as_str()),
        Some("*log*.md")
    );
    assert_eq!(
        rewrite.runtime_args.get("root").and_then(|v| v.as_str()),
        Some(".")
    );
}

#[test]
fn fs_basic_find_entries_pure_glob_name_pattern_rewrites_to_ext_search() {
    let args = json!({
        "action": "find_entries",
        "path": "/repo",
        "target_kind": "file",
        "name_pattern": "*.sh"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_ext")
    );
    assert_eq!(
        rewrite.runtime_args.get("ext").and_then(|v| v.as_str()),
        Some("sh")
    );
    assert_eq!(
        rewrite.runtime_args.get("root").and_then(|v| v.as_str()),
        Some("/repo")
    );
    assert!(rewrite.runtime_args.get("pattern").is_none());
}

#[test]
fn fs_basic_find_entries_concrete_name_pattern_requests_exact_match() {
    let args = json!({
        "action": "find_entries",
        "path": "scripts/nl_tests/fixtures/locator_smart",
        "target_kind": "file",
        "name_pattern": "Report.MD"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_name")
    );
    assert_eq!(
        rewrite.runtime_args.get("pattern").and_then(|v| v.as_str()),
        Some("Report.MD")
    );
    assert_eq!(
        rewrite.runtime_args.get("exact").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn fs_basic_find_entries_max_entries_rewrites_to_fs_search_max_results() {
    let args = json!({
        "action": "find_entries",
        "root": "scripts/nl_tests/fixtures/locator_smart",
        "target_kind": "file",
        "pattern": "*abcd*",
        "max_entries": 4
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_name")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("max_results")
            .and_then(|v| v.as_u64()),
        Some(4)
    );
    assert!(rewrite.runtime_args.get("max_entries").is_none());
}

#[test]
fn fs_basic_find_entries_entry_name_alias_rewrites_to_name_search() {
    let args = json!({
        "action": "find_entries",
        "target_path": "scripts/nl_tests/fixtures/device_local/tmp",
        "entry_name": "config.ini",
        "target_kind": "file"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_name")
    );
    assert_eq!(
        rewrite.runtime_args.get("root").and_then(|v| v.as_str()),
        Some("scripts/nl_tests/fixtures/device_local/tmp")
    );
    assert_eq!(
        rewrite.runtime_args.get("pattern").and_then(|v| v.as_str()),
        Some("config.ini")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("target_kind")
            .and_then(|v| v.as_str()),
        Some("file")
    );
}

#[test]
fn fs_basic_find_entries_extensions_alias_rewrites_to_ext_search() {
    let args = json!({
        "action": "find_entries",
        "root": ".",
        "extensions": ["md", "txt"],
        "pattern": "log"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("find_ext")
    );
    assert_eq!(rewrite.runtime_args.get("ext"), Some(&json!(["md", "txt"])));
}

#[test]
fn fs_basic_grep_text_drops_redundant_query_pattern_before_runtime() {
    let args = json!({
        "action": "grep_text",
        "root": ".",
        "query": "FirstLayerDecision",
        "pattern": "FirstLayerDecision"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("grep_text")
    );
    assert_eq!(
        rewrite.runtime_args.get("query").and_then(|v| v.as_str()),
        Some("FirstLayerDecision")
    );
    assert!(rewrite.runtime_args.get("pattern").is_none());
}

#[test]
fn fs_basic_grep_text_promotes_single_pattern_to_content_query() {
    let args = json!({
        "action": "grep_text",
        "path": ".",
        "pattern": "FirstLayerDecision"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite.runtime_args.get("query").and_then(|v| v.as_str()),
        Some("FirstLayerDecision")
    );
    assert!(rewrite.runtime_args.get("pattern").is_none());
    assert_eq!(
        rewrite.runtime_args.get("root").and_then(|v| v.as_str()),
        Some(".")
    );
}

#[test]
fn fs_basic_find_entries_without_criterion_degrades_to_directory_listing() {
    let args = json!({"action":"find_entries", "path":"plan", "target_kind":"file"});
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("inventory_dir")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("plan")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("files_only")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn fs_basic_find_entries_wildcard_only_degrades_to_directory_listing() {
    let args = json!({
        "action": "find_entries",
        "root": "/home/guagua/rustclaw/plan",
        "pattern": "*",
        "target_kind": "file",
        "max_results": 50
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("inventory_dir")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("/home/guagua/rustclaw/plan")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("files_only")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("max_entries")
            .and_then(|v| v.as_u64()),
        Some(50)
    );
    assert!(rewrite.runtime_args.get("pattern").is_none());
}

#[test]
fn fs_basic_find_entries_mixed_patterns_keep_literal_selectors() {
    let args = json!({
        "action": "find_entries",
        "root": "plan",
        "pattern": ["*", "runtime"],
        "target_kind": "file"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "fs_search");
    assert_eq!(
        rewrite
            .runtime_args
            .get("pattern")
            .and_then(|value| value.as_array()),
        Some(&vec![json!("runtime")])
    );
}

#[test]
fn fs_basic_find_entries_existing_directory_pattern_degrades_to_listing() {
    let dir = std::env::temp_dir().join(format!(
        "rustclaw-fs-basic-pattern-dir-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp directory");
    let args = json!({
        "action": "find_entries",
        "root": ".",
        "pattern": dir.to_string_lossy(),
        "target_kind": "file"
    });
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("inventory_dir")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        dir.to_str()
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("files_only")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(rewrite.runtime_args.get("pattern").is_none());
}

#[test]
fn fs_basic_append_text_rewrites_to_append_write_file() {
    let mut args = json!({"action":"append_line", "file":"memo.txt", "text":"beta\n"});
    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "write_file");
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("memo.txt")
    );
    assert_eq!(
        rewrite.runtime_args.get("content").and_then(|v| v.as_str()),
        Some("beta\n")
    );
    assert_eq!(
        rewrite.runtime_args.get("append").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(rewrite.runtime_args.get("action").is_none());
}

#[test]
fn fs_basic_write_text_write_mode_alias_rewrites_to_write_file_mode() {
    let mut args = json!({
        "action": "write_text",
        "path": "memo.txt",
        "content": "new\n",
        "write_mode": "overwrite"
    });
    assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
    let rewrite = rewrite_virtual_tool_call("fs_basic", args)
        .unwrap()
        .expect("rewrite");

    assert_eq!(rewrite.runtime_tool, "write_file");
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("memo.txt")
    );
    assert_eq!(
        rewrite.runtime_args.get("content").and_then(|v| v.as_str()),
        Some("new\n")
    );
    assert_eq!(
        rewrite.runtime_args.get("mode").and_then(|v| v.as_str()),
        Some("overwrite")
    );
    assert!(rewrite.runtime_args.get("write_mode").is_none());
    assert!(rewrite.runtime_args.get("action").is_none());
}

#[test]
fn fs_basic_patch_actions_rewrite_to_workspace_patch() {
    for action in [
        "apply_patch",
        "diff",
        "rewind",
        "review_child_patch",
        "apply_child_patch",
        "reject_child_patch",
    ] {
        let args = json!({"action": action, "checkpoint_id": "patch_12345678"});
        let rewrite = rewrite_virtual_tool_call("fs_basic", args.clone())
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "workspace_patch");
        assert_eq!(rewrite.runtime_args, args);
    }
}

#[test]
fn config_basic_read_field_rewrites_to_system_basic_extract_field() {
    let mut args =
        json!({"action":"extract_field", "file":"Cargo.toml", "key":"workspace.package.version"});
    assert!(normalize_virtual_tool_arg_aliases(
        "config_basic",
        &mut args
    ));
    let rewrite = rewrite_virtual_tool_call("config_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("extract_field")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("Cargo.toml")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("field_path")
            .and_then(|v| v.as_str()),
        Some("workspace.package.version")
    );
}

#[test]
fn config_basic_missing_action_with_field_path_defaults_to_read_field() {
    let mut args = json!({"path":"/tmp/package.json", "field_path":"name", "format":"json"});
    assert!(normalize_virtual_tool_arg_aliases(
        "config_basic",
        &mut args
    ));
    let rewrite = rewrite_virtual_tool_call("config_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("extract_field")
    );
    assert_eq!(
        rewrite
            .runtime_args
            .get("field_path")
            .and_then(|v| v.as_str()),
        Some("name")
    );
}

#[test]
fn config_basic_guard_rewrites_to_config_edit_guard() {
    let args = json!({"action":"guard_rustclaw_config"});
    let rewrite = rewrite_virtual_tool_call("config_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "config_edit");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("guard_config")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("configs/config.toml")
    );
}

#[test]
fn config_basic_validate_rewrites_to_structured_validation() {
    let args = json!({"action":"validate", "path":"configs/config.toml", "format":"toml"});
    let rewrite = rewrite_virtual_tool_call("config_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "system_basic");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("validate_structured")
    );
    assert_eq!(
        rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
        Some("configs/config.toml")
    );
}

#[test]
fn config_basic_semantic_guard_profile_rewrites_to_config_guard() {
    let args = json!({
        "action":"validate",
        "path":"configs/config.toml",
        "validation_profile":"rustclaw_semantic_guard"
    });
    let rewrite = rewrite_virtual_tool_call("config_basic", args)
        .unwrap()
        .expect("rewrite");
    assert_eq!(rewrite.runtime_tool, "config_edit");
    assert_eq!(
        rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
        Some("guard_config")
    );
    assert!(rewrite.runtime_args.get("validation_profile").is_none());
}
