use super::{
    do_ingest, do_list_namespaces, do_stats, error_extra, normalize_search_path_prefix,
    parse_ingest_args, parse_stats_args, split_chunks, storage_path_for, tokenize, KbRuntime,
    SKILL_NAME,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn runtime(root: &std::path::Path, user_key: &str) -> KbRuntime {
    KbRuntime {
        scope_user_key: user_key.to_string(),
        workspace_root: root.to_path_buf(),
        storage_database_path: root.join("data/skills/kb/state.db"),
        storage_busy_timeout_ms: 5_000,
    }
}

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.kb.execution_failed");
    assert_eq!(extra["retryable"], false);
}

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
fn ingest_success_extra_includes_path_evidence_fields() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_kb_ingest_path_evidence_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp kb workspace");
    fs::write(
        root.join("README.md"),
        "# Demo\n\nThis document is indexed for a knowledge-base ingest test.",
    )
    .expect("write README fixture");
    let runtime = runtime(&root, "user:test");

    let out = do_ingest(
        &runtime,
        &json!({
            "action": "ingest",
            "namespace": "demo_docs_nl",
            "paths": ["README.md"],
            "overwrite": true
        }),
    )
    .expect("ingest succeeds");

    assert_eq!(
        out.get("path").and_then(|value| value.as_str()),
        Some("README.md")
    );
    assert_eq!(
        out.get("action").and_then(|value| value.as_str()),
        Some("ingest")
    );
    assert_eq!(
        out.get("paths")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.as_str()),
        Some("README.md")
    );
    assert_eq!(
        out.pointer("/stats/ingested_docs")
            .and_then(|value| value.as_u64()),
        Some(1)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn list_namespaces_extra_includes_names_and_count_fields() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_kb_list_namespaces_fields_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp kb workspace");
    fs::write(
        root.join("README.md"),
        "# Demo\n\nThis document is indexed for a knowledge-base namespace listing test.",
    )
    .expect("write README fixture");
    let runtime = runtime(&root, "user:test");

    do_ingest(
        &runtime,
        &json!({
            "action": "ingest",
            "namespace": "demo_docs_nl",
            "paths": ["README.md"],
            "overwrite": true
        }),
    )
    .expect("ingest succeeds");
    let out = do_list_namespaces(&runtime).expect("list namespaces succeeds");

    assert_eq!(out.get("count").and_then(|value| value.as_u64()), Some(1));
    assert_eq!(
        out.get("namespace_count").and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        out.get("names")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.as_str()),
        Some("demo_docs_nl")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn stats_extra_includes_document_and_chunk_count_aliases() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_kb_stats_count_aliases_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp kb workspace");
    fs::write(
        root.join("README.md"),
        "# Demo\n\nThis document is indexed for a knowledge-base stats test.",
    )
    .expect("write README fixture");
    let runtime = runtime(&root, "user:test");

    do_ingest(
        &runtime,
        &json!({
            "action": "ingest",
            "namespace": "demo_docs_nl",
            "paths": ["README.md"],
            "overwrite": true
        }),
    )
    .expect("ingest succeeds");
    let out = do_stats(&runtime, &json!({"namespace": "demo_docs_nl"})).expect("stats succeeds");

    assert_eq!(
        out.get("namespace").and_then(|value| value.as_str()),
        Some("demo_docs_nl")
    );
    assert_eq!(
        out.get("document_count").and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        out.get("chunk_count").and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        out.pointer("/stats/document_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        out.pointer("/stats/chunk_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_unchanged_file_marks_idempotent_success() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_kb_ingest_idempotent_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp kb workspace");
    fs::write(root.join("README.md"), "# Demo\n\nIndexed content.").expect("write README fixture");
    let runtime = runtime(&root, "user:test");
    let args = json!({
        "action": "ingest",
        "namespace": "demo_docs_nl",
        "paths": ["README.md"]
    });

    let first = do_ingest(&runtime, &args).expect("first ingest succeeds");
    let second = do_ingest(&runtime, &args).expect("second ingest succeeds");

    assert_eq!(
        first.get("result_kind").and_then(|value| value.as_str()),
        Some("updated")
    );
    assert_eq!(
        second
            .pointer("/stats/ingested_docs")
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        second
            .get("effective_status")
            .and_then(|value| value.as_str()),
        Some("ok")
    );
    assert_eq!(
        second.get("result_kind").and_then(|value| value.as_str()),
        Some("already_indexed")
    );
    assert_eq!(
        second.get("summary").and_then(|value| value.as_str()),
        Some("already_indexed")
    );
    assert_eq!(
        second
            .get("idempotent_success")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        second
            .get("effective_success")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tokenize_supports_cjk_queries() {
    let terms = tokenize("基础健康检查");
    assert!(terms.contains(&"基础".to_string()));
    assert!(terms.contains(&"健康".to_string()));
}

#[test]
fn runtime_uses_the_kb_owned_database_path() {
    let runtime = runtime(&PathBuf::from("/tmp/workspace"), "user:alpha");
    assert_eq!(
        runtime.storage_database_path,
        PathBuf::from("/tmp/workspace/data/skills/kb/state.db")
    );
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
