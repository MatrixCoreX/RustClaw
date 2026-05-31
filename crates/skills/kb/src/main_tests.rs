use super::{
    normalize_search_path_prefix, parse_ingest_args, parse_stats_args, split_chunks,
    storage_path_for, storage_segment, tokenize, KbRuntime,
};
use serde_json::json;
use std::path::PathBuf;

#[test]
fn split_chunks_keeps_overlap_context() {
    let text = "# Title\nFirst paragraph talks about deployment.\n\nSecond paragraph keeps going with more details.";
    let chunks = split_chunks(text, 40, 10);
    assert!(chunks.len() >= 2);
    assert!(chunks[1].contains("deployment") || chunks[1].contains("paragraph"));
}

#[test]
fn stats_args_accept_optional_namespace() {
    let scoped = parse_stats_args(&json!({"namespace":"docs"})).expect("parse scoped stats");
    assert_eq!(scoped.namespace.as_deref(), Some("docs"));

    let global = parse_stats_args(&json!({})).expect("parse global stats");
    assert!(global.namespace.is_none());
}

#[test]
fn ingest_args_accept_single_path_alias() {
    let parsed = parse_ingest_args(&json!({
        "namespace": "docs",
        "path": "README.md"
    }))
    .expect("parse single path alias");

    assert_eq!(parsed.paths, vec!["README.md"]);
}

#[test]
fn tokenize_supports_cjk_queries() {
    let terms = tokenize("基础健康检查");
    assert!(terms.contains(&"基础".to_string()));
    assert!(terms.contains(&"健康".to_string()));
}

#[test]
fn storage_segment_is_stable_and_hashed() {
    let first = storage_segment("docs/release notes");
    let second = storage_segment("docs/release notes");
    assert_eq!(first, second);
    assert!(first.contains("--"));
}

#[test]
fn kb_root_is_user_scoped() {
    let runtime = KbRuntime {
        scope_user_key: "user:alpha".to_string(),
        workspace_root: PathBuf::from("/tmp/workspace"),
        unified_index_db_path: None,
        unified_index_busy_timeout_ms: None,
    };
    let root = super::kb_root(&runtime);
    assert!(root.starts_with(PathBuf::from("/tmp/workspace/data/kb/by_user")));
    assert!(root.file_name().is_some());
}

#[test]
fn storage_path_prefers_workspace_relative_paths() {
    let workspace = PathBuf::from("/tmp/workspace");
    let file = workspace.join("document/manual_note.txt");
    assert_eq!(
        storage_path_for(&file, &workspace),
        "document/manual_note.txt"
    );
}

#[test]
fn normalize_search_prefix_converts_absolute_workspace_prefix_to_relative() {
    let workspace = PathBuf::from("/tmp/workspace");
    let prefix = workspace.join("document");
    assert_eq!(
        normalize_search_path_prefix(&workspace, &prefix.display().to_string()),
        "document"
    );
}
