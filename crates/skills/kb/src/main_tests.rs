use super::{
    build_scan_targets, default_chunker_version, default_parser_version, do_delete_namespace,
    do_ingest, do_list_documents, do_list_namespaces, do_reindex, do_remove_documents, do_search,
    do_stats, document_is_current, error_extra, normalize_search_path_prefix, parse_ingest_args,
    parse_stats_args, split_chunks, storage_path_for, tokenize, DocMeta, KbRuntime, SKILL_NAME,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn runtime(root: &std::path::Path, user_key: &str) -> KbRuntime {
    fs::create_dir_all(root).expect("create KB test workspace");
    KbRuntime {
        scope_user_key: user_key.to_string(),
        workspace_root: root.to_path_buf(),
        storage_database_path: root.join("data/skills/kb/state.db"),
        storage_busy_timeout_ms: 5_000,
        path_policy: skill_sdk::SkillPathPolicy::new(root, None)
            .expect("create KB test path policy"),
    }
}

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "execution_failed");
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
        "agent_kb_ingest_path_evidence_{}",
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
        "agent_kb_list_namespaces_fields_{}",
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
        "agent_kb_stats_count_aliases_{}",
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
    let root =
        std::env::temp_dir().join(format!("agent_kb_ingest_idempotent_{}", std::process::id()));
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

#[test]
fn content_digest_not_mtime_and_size_controls_incremental_identity() {
    let doc = DocMeta {
        path: "guide.md".to_string(),
        file_type: "md".to_string(),
        mtime_epoch: 7,
        size: 5,
        chunks: 1,
        content_sha256: super::sha256_hex(b"alpha"),
        parser_version: default_parser_version(),
        chunker_version: default_chunker_version(),
    };
    assert!(document_is_current(
        &doc,
        &super::sha256_hex(b"alpha"),
        &default_parser_version(),
        &default_chunker_version()
    ));
    assert!(!document_is_current(
        &doc,
        &super::sha256_hex(b"bravo"),
        &default_parser_version(),
        &default_chunker_version()
    ));
}

#[test]
fn ingest_job_resumes_from_persisted_checkpoint() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-resumable-ingest-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha resumable document").expect("a fixture");
    fs::write(root.join("b.md"), "beta resumable document").expect("b fixture");
    let runtime = runtime(&root, "user:resume");

    let first = do_ingest(
        &runtime,
        &json!({
            "namespace": "docs",
            "paths": ["a.md", "b.md"],
            "max_files_per_run": 1
        }),
    )
    .expect("start bounded ingest");
    assert_eq!(first["complete"], false);
    assert_eq!(first["job_status"], "waiting");
    assert_eq!(first["stats"]["job_processed_files"], 1);
    let job_id = first["job_id"].as_str().expect("job id").to_string();

    let resumed = super::do_resume_ingest(&runtime, &json!({"job_id": job_id}))
        .expect("resume persisted ingest");
    assert_eq!(resumed["complete"], true);
    assert_eq!(resumed["job_status"], "completed");
    assert_eq!(resumed["stats"]["job_processed_files"], 2);
    assert_eq!(resumed["stats"]["total_docs"], 2);

    let status = super::do_ingest_job_status(&runtime, &json!({"job_id": job_id}))
        .expect("load persisted job status");
    assert_eq!(status["job_status"], "completed");
    assert_eq!(status["progress"]["processed_files"], 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_job_cancel_is_owner_scoped() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-cancel-ingest-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha").expect("a fixture");
    fs::write(root.join("b.md"), "beta").expect("b fixture");
    let owner = runtime(&root, "user:owner");
    let other = runtime(&root, "user:other");
    let first = do_ingest(
        &owner,
        &json!({
            "namespace": "docs",
            "paths": ["a.md", "b.md"],
            "max_files_per_run": 1
        }),
    )
    .expect("start ingest");
    let job_id = first["job_id"].as_str().expect("job id");
    assert!(super::do_ingest_job_status(&other, &json!({"job_id": job_id})).is_err());
    let cancelled =
        super::do_cancel_ingest(&owner, &json!({"job_id": job_id})).expect("cancel owned job");
    assert_eq!(cancelled["job_status"], "cancelled");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn path_policy_confines_regular_users_and_allows_verified_admin_absolute_sources() {
    let base = std::env::temp_dir().join(format!(
        "agent-runtime-kb-path-policy-{}",
        std::process::id()
    ));
    let workspace = base.join("workspace");
    let external = base.join("external.md");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(&external, "external knowledge").expect("external fixture");
    let regular = runtime(&workspace, "user:regular");
    let external_text = external.display().to_string();
    assert!(build_scan_targets(&regular, std::slice::from_ref(&external_text)).is_err());

    let host_grant_context = json!({
        "authority_scope": "host_policy_grant",
        "permissions": {
            "allow_path_outside_workspace": true
        }
    });
    let admin = KbRuntime {
        scope_user_key: "user:admin".to_string(),
        workspace_root: workspace.clone(),
        storage_database_path: workspace.join("data/skills/kb/state.db"),
        storage_busy_timeout_ms: 5_000,
        path_policy: skill_sdk::SkillPathPolicy::new(&workspace, Some(&host_grant_context))
            .expect("host path policy"),
    };
    let targets = build_scan_targets(&admin, &[external_text]).expect("admin external source");
    assert_eq!(targets.len(), 1);
    assert_eq!(
        targets[0].root,
        external.canonicalize().expect("canonical external")
    );
    let _ = fs::remove_dir_all(base);
}

#[test]
fn document_management_actions_are_structured_and_transactional() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-document-management-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha document").expect("a fixture");
    fs::write(root.join("b.md"), "beta document").expect("b fixture");
    let runtime = runtime(&root, "user:test");
    do_ingest(
        &runtime,
        &json!({
            "namespace": "docs",
            "paths": ["a.md", "b.md"]
        }),
    )
    .expect("ingest documents");

    let listed =
        do_list_documents(&runtime, &json!({"namespace": "docs"})).expect("list documents");
    assert_eq!(listed["document_count"], 2);
    assert!(listed["namespace_revision"].as_u64().unwrap_or_default() > 0);
    assert!(listed["documents"][0]["content_sha256"]
        .as_str()
        .is_some_and(|digest| digest.len() == 64));

    let search = do_search(
        &runtime,
        &json!({"namespace": "docs", "query": "alpha", "top_k": 2}),
    )
    .expect("search normalized index");
    assert_eq!(search["stats"]["retrieval_mode"], "fts5_candidates");
    assert_eq!(search["stats"]["total_candidates"], 2);

    let removed = do_remove_documents(&runtime, &json!({"namespace": "docs", "paths": ["a.md"]}))
        .expect("remove document");
    assert_eq!(removed["removed_count"], 1);
    assert_eq!(removed["remaining_documents"], 1);

    fs::write(root.join("b.md"), "beta document updated").expect("update b fixture");
    let reindexed = do_reindex(&runtime, &json!({"namespace": "docs"})).expect("reindex namespace");
    assert_eq!(reindexed["action"], "reindex");
    assert_eq!(reindexed["stats"]["total_docs"], 1);

    let deleted =
        do_delete_namespace(&runtime, &json!({"namespace": "docs"})).expect("delete namespace");
    assert_eq!(deleted["deleted"], true);
    assert_eq!(deleted["cleanup_status"], "cleaned");
    assert_eq!(deleted["removed_documents"], 1);
    assert!(do_list_documents(&runtime, &json!({"namespace": "docs"})).is_err());
    let _ = fs::remove_dir_all(root);
}
