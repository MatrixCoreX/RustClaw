use super::{
    build_scan_targets, default_chunker_version, default_parser_version, do_delete_namespace,
    do_ingest, do_list_documents, do_list_namespaces, do_reindex, do_remove_documents, do_search,
    do_stats, document_is_current, error_extra, normalize_search_path_prefix, parse_ingest_args,
    parse_search_args, parse_stats_args, split_chunks, storage_path_for, tokenize, DocMeta,
    KbRuntime, SKILL_NAME,
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
        storage_resource_profile: "standard".to_string(),
        storage_cache_size_kib: None,
        storage_mmap_size_bytes: None,
        storage_temp_store: None,
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
fn vector_projection_degradation_returns_a_structured_maintenance_action() {
    for error_code in ["embedding_projection_stale", "ann_projection_stale"] {
        let maintenance = super::vector_maintenance_value("docs", Some(error_code));
        assert_eq!(maintenance["required"], true);
        assert_eq!(maintenance["error_code"], error_code);
        assert_eq!(maintenance["message_key"], format!("skill.kb.{error_code}"));
        assert_eq!(
            maintenance["suggested_action"],
            json!({"action":"start_vector_reindex", "namespace":"docs"})
        );
    }
    assert_eq!(
        super::vector_maintenance_value("docs", None)["required"],
        false
    );
}

#[test]
fn hybrid_search_merges_adjacent_chunks_and_prioritizes_document_diversity() {
    fn hit(
        document_id: &str,
        path: &str,
        chunk_id: &str,
        offset: u64,
        score: f64,
    ) -> super::SearchHit {
        super::SearchHit {
            schema_version: 2,
            evidence_id: claw_core::retrieval_embedding::retrieval_evidence_id(
                &format!("kb://docs/{document_id}/{chunk_id}"),
                &format!("digest-{document_id}"),
                1,
            ),
            source_ref: format!("kb://docs/{document_id}/{chunk_id}"),
            namespace: "docs".to_string(),
            namespace_revision: 1,
            document_id: document_id.to_string(),
            document_digest: format!("digest-{document_id}"),
            chunk_id: chunk_id.to_string(),
            chunk_ids: vec![chunk_id.to_string()],
            path: path.to_string(),
            file_type: "md".to_string(),
            location: json!({"path":path, "offset":offset}),
            text: format!("evidence {chunk_id}"),
            score: super::SearchScore {
                final_score: score,
                lexical: score,
                semantic: score,
                freshness: 0.0,
                authority: 1.0,
                rerank: score,
            },
            retrieval_modes: vec!["lexical", "semantic"],
            instruction_authority: "none",
            hit_terms: vec!["evidence".to_string()],
            metadata: json!({"path":path, "offset":offset}),
        }
    }

    assert!((super::stable_hybrid_score(0.8, 0.4) - 0.68).abs() < f64::EPSILON);
    assert!((super::calibrated_semantic_only_score(0.8) - 0.6).abs() < f64::EPSILON);
    let mut merged = super::merge_adjacent_search_hits(vec![
        hit("doc-a", "a.md", "a-1", 0, 0.9),
        hit("doc-a", "a.md", "a-2", 1, 0.8),
        hit("doc-a", "a.md", "a-4", 4, 0.7),
        hit("doc-b", "b.md", "b-1", 0, 0.75),
    ]);
    assert_eq!(merged.len(), 3);
    let adjacent = merged
        .iter()
        .find(|candidate| candidate.chunk_ids.len() == 2)
        .expect("adjacent evidence group");
    assert_eq!(adjacent.chunk_ids, vec!["a-1", "a-2"]);
    assert_eq!(adjacent.location["offset"], 0);
    assert_eq!(adjacent.location["end_offset"], 1);
    assert_eq!(adjacent.metadata["merged_adjacent"], true);
    assert!(adjacent.evidence_id.starts_with("evidence:sha256:"));

    merged.sort_by(|left, right| {
        right
            .score
            .final_score
            .partial_cmp(&left.score.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let selected = super::select_document_diverse_hits(merged, 2);
    assert_eq!(selected.len(), 2);
    assert_ne!(selected[0].document_id, selected[1].document_id);
}

#[test]
fn split_chunks_keeps_overlap_context() {
    let text = "# Title\nFirst paragraph talks about deployment.\n\nSecond paragraph keeps going with more details.";
    let chunks = split_chunks(text, 40, 10);
    assert!(chunks.len() >= 2);
    assert!(chunks[1].contains("deployment") || chunks[1].contains("paragraph"));
}

#[test]
fn split_chunks_prefers_sentence_or_line_boundaries_before_character_windows() {
    let text = "First complete sentence. Second complete sentence.\nThird complete line. Fourth complete sentence.";
    let chunks = split_chunks(text, 55, 5);

    assert!(chunks.len() >= 2);
    assert!(chunks[0].ends_with('.') || chunks[0].ends_with('\n'));
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 55));
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
fn constrained_runtime_uses_smaller_checkpointed_defaults_without_rejecting_overrides() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-constrained-profile-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    let mut runtime = runtime(&root, "user:constrained");
    runtime.storage_resource_profile = "constrained".to_string();

    let defaults = super::ingest::parse_ingest_args_for_runtime(
        &runtime,
        &json!({"namespace":"docs", "paths":["a.md"]}),
    )
    .expect("constrained defaults");
    assert_eq!(defaults.max_files_per_run, 64);
    assert_eq!(defaults.max_bytes_per_run, 16 * 1024 * 1024);
    assert_eq!(defaults.max_chunks_per_run, 2_048);
    assert_eq!(defaults.max_run_seconds, 10);
    assert_eq!(
        super::vector_reindex_batch_size(&runtime, &json!({})).unwrap(),
        8
    );
    assert_eq!(runtime.active_job_limit(), 1);

    let explicit = super::ingest::parse_ingest_args_for_runtime(
        &runtime,
        &json!({
            "namespace":"docs",
            "paths":["a.md"],
            "max_files_per_run":5000,
            "max_bytes_per_run":268435456,
            "max_chunks_per_run":10000,
            "max_run_seconds":300
        }),
    )
    .expect("valid explicit overrides");
    assert_eq!(explicit.max_files_per_run, 5_000);
    assert_eq!(explicit.max_bytes_per_run, 256 * 1024 * 1024);
    assert_eq!(explicit.max_chunks_per_run, 10_000);
    assert_eq!(explicit.max_run_seconds, 300);
    assert_eq!(
        super::vector_reindex_batch_size(&runtime, &json!({"batch_size":256})).unwrap(),
        256
    );

    let paths = (0..65)
        .map(|index| {
            let path = format!("{index:03}.md");
            fs::write(root.join(&path), format!("document {index}")).expect("constrained fixture");
            path
        })
        .collect::<Vec<_>>();
    let waiting = do_ingest(&runtime, &json!({"namespace":"docs", "paths":paths}))
        .expect("bounded constrained ingestion");
    assert_eq!(waiting["job_status"], "waiting");
    assert_eq!(waiting["stats"]["processed_files"], 64);
    assert_eq!(waiting["continuation"]["next_file_index"], 64);
    super::do_cancel_ingest(
        &runtime,
        &json!({"job_id":waiting["job_id"].as_str().expect("job id")}),
    )
    .expect("cancel constrained fixture");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn direct_search_contract_rejects_out_of_bounds_values() {
    let oversized_query = "x".repeat(8_193);
    for args in [
        json!({"namespace":"docs", "query": oversized_query}),
        json!({"namespace":"docs", "query":"alpha", "top_k": 0}),
        json!({"namespace":"docs", "query":"alpha", "top_k": 51}),
        json!({"namespace":"docs", "query":"alpha", "top_k": "5"}),
        json!({"namespace":"docs", "query":"alpha", "min_score": -0.1}),
        json!({"namespace":"docs", "query":"alpha", "min_score": "0.5"}),
        json!({"namespace":"docs", "query":"alpha", "time_from": "not-an-epoch"}),
        json!({"namespace":"docs", "query":"alpha", "time_from": 20, "time_to": 10}),
        json!({"namespace":"docs", "query":"alpha", "filters": []}),
        json!({"namespace":"docs", "query":"alpha", "filters": {"owner_user_key":"rk-other"}}),
        json!({"namespace":"docs", "query":"alpha", "filters": {"path_prefix":"x".repeat(4097)}}),
        json!({"namespace":"docs", "query":"alpha", "path_prefix": "bad\npath"}),
    ] {
        assert!(
            parse_search_args(&args).is_err(),
            "accepted invalid args: {args}"
        );
    }

    assert!(parse_search_args(&json!({
        "namespace":"docs",
        "query":"alpha",
        "top_k": 50,
        "min_score": 0.5,
        "time_from": "10",
        "time_to": 20
    }))
    .is_ok());
}

#[test]
fn kb_actions_keep_namespace_documents_search_and_jobs_owner_scoped() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-owner-boundary-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("alpha.md"), "alpha-visible owner material").expect("alpha fixture");
    fs::write(root.join("beta.md"), "beta-private cross tenant secret").expect("beta fixture");
    let alpha = runtime(&root, "rk-alpha");
    let beta = runtime(&root, "rk-beta");

    do_ingest(
        &alpha,
        &json!({"namespace":"shared-name", "paths":["alpha.md"]}),
    )
    .expect("alpha ingest");
    let beta_job = do_ingest(
        &beta,
        &json!({"namespace":"shared-name", "paths":["beta.md"]}),
    )
    .expect("beta ingest");

    let alpha_documents =
        do_list_documents(&alpha, &json!({"namespace":"shared-name"})).expect("alpha list");
    assert_eq!(alpha_documents["document_count"], 1);
    assert_eq!(alpha_documents["documents"][0]["path"], "alpha.md");
    let alpha_search = do_search(
        &alpha,
        &json!({"namespace":"shared-name", "query":"cross tenant secret", "top_k":10}),
    )
    .expect("alpha search");
    assert_eq!(alpha_search["hits"], json!([]));
    assert!(!alpha_search.to_string().contains("beta-private"));

    let beta_documents =
        do_list_documents(&beta, &json!({"namespace":"shared-name"})).expect("beta list");
    assert_eq!(beta_documents["document_count"], 1);
    assert_eq!(beta_documents["documents"][0]["path"], "beta.md");
    let beta_search = do_search(
        &beta,
        &json!({"namespace":"shared-name", "query":"cross tenant secret", "top_k":10}),
    )
    .expect("beta search");
    assert_eq!(beta_search["hits"][0]["location"]["path"], "beta.md");

    let beta_job_id = beta_job["job_id"].as_str().expect("beta job id");
    assert!(super::do_ingest_job_status(&alpha, &json!({"job_id":beta_job_id})).is_err());
    assert!(super::do_ingest_job_status(&beta, &json!({"job_id":beta_job_id})).is_ok());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn direct_ingest_contract_rejects_out_of_bounds_values() {
    let too_many_paths = (0..257)
        .map(|index| format!("{index}.md"))
        .collect::<Vec<_>>();
    for args in [
        json!({"namespace":"docs", "paths": too_many_paths}),
        json!({"namespace":"docs", "paths": ["bad\npath.md"]}),
        json!({"namespace":"docs", "paths": [42]}),
        json!({"namespace":"docs", "paths": ["a.md"], "chunk_size": 199}),
        json!({"namespace":"docs", "paths": ["a.md"], "chunk_overlap": 401}),
        json!({"namespace":"docs", "paths": ["a.md"], "chunk_size": 300, "chunk_overlap": 101}),
        json!({"namespace":"docs", "paths": ["a.md"], "file_types": [""]}),
        json!({"namespace":"docs", "paths": ["a.md"], "max_file_size": 67108865}),
        json!({"namespace":"docs", "paths": ["a.md"], "max_files_per_run": 5001}),
        json!({"namespace":"docs", "paths": ["a.md"], "max_bytes_per_run": 268435457}),
        json!({"namespace":"docs", "paths": ["a.md"], "max_chunks_per_run": 10001}),
        json!({"namespace":"docs", "paths": ["a.md"], "max_run_seconds": 301}),
    ] {
        assert!(
            parse_ingest_args(&args).is_err(),
            "accepted invalid args: {args}"
        );
    }

    assert!(parse_ingest_args(&json!({
        "namespace":"docs",
        "paths": ["a.md"],
        "chunk_size": 200
    }))
    .is_ok());
}

#[test]
fn direct_list_stats_and_job_contracts_enforce_boundaries() {
    let root = std::env::temp_dir().join(format!(
        "agent_kb_direct_contract_boundaries_{}",
        std::process::id()
    ));
    let runtime = runtime(&root, "user:test");

    assert!(do_list_documents(&runtime, &json!({"namespace":"docs", "limit": 0})).is_err());
    assert!(do_list_documents(
        &runtime,
        &json!({"namespace":"docs", "continuation": "not-an-object"})
    )
    .is_err());
    assert!(parse_stats_args(&json!({"namespace": "bad\nnamespace"})).is_err());
    assert!(
        super::ingest::do_ingest_job_status(&runtime, &json!({"job_id": "x".repeat(129)})).is_err()
    );

    let _ = fs::remove_dir_all(root);
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
    assert_eq!(
        out.pointer("/namespaces/0/active_vector_generation")
            .and_then(|value| value.as_u64()),
        Some(1)
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
        Some(2)
    );
    assert_eq!(
        out.pointer("/stats/document_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        out.pointer("/stats/chunk_count")
            .and_then(|value| value.as_u64()),
        Some(2)
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
fn tokenize_indexes_unique_cjk_terms_at_the_end_of_long_text() {
    let terms =
        tokenize("这是一个足够长的中文知识文档用于验证完整索引不会只保留开头内容最终答案藏在尾部");
    assert!(terms.contains(&"尾部".to_string()));
    assert!(terms.contains(&"答案".to_string()));
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
fn ingest_registry_actions_remain_local_mutations_in_all_distributions() {
    use claw_core::skill_registry::{
        CapabilityIsolationProfile, PlannerCapabilityEffect, SkillsRegistry,
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for relative in [
        "configs/skills_registry.toml",
        "docker/config/skills_registry.toml",
    ] {
        let registry = SkillsRegistry::load_from_path(&root.join(relative)).expect("registry");
        for action in [
            "ingest",
            "reindex",
            "resume_ingest",
            "start_vector_reindex",
            "resume_vector_reindex",
            "cancel_vector_reindex",
        ] {
            let mapping = registry
                .planner_capabilities("kb")
                .iter()
                .find(|mapping| mapping.action.as_deref() == Some(action))
                .expect("KB action mapping");
            assert_eq!(
                mapping.effect,
                Some(PlannerCapabilityEffect::Mutate),
                "{relative}: {action}"
            );
            assert_eq!(
                mapping.isolation_profile,
                Some(CapabilityIsolationProfile::LocalCurrentWorkspace)
            );
            assert_eq!(mapping.network_access, Some(false));
            assert_eq!(mapping.filesystem_write, Some(true));
        }
        assert!(registry.resolved_once_per_task("kb", Some("resume_ingest")));
        assert!(!registry.resolved_idempotent("kb", Some("resume_ingest")));
    }
}

#[test]
fn vector_reindex_actions_expose_resumable_machine_checkpoints() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-vector-reindex-actions-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha vector document").expect("a fixture");
    fs::write(root.join("b.md"), "beta vector document").expect("b fixture");
    fs::write(root.join("c.md"), "gamma vector document").expect("c fixture");
    let runtime = runtime(&root, "user:vector-reindex");
    do_ingest(
        &runtime,
        &json!({
            "namespace": "docs",
            "paths": ["a.md", "b.md", "c.md"]
        }),
    )
    .expect("ingest vector source");

    let first =
        super::do_start_vector_reindex(&runtime, &json!({"namespace":"docs", "batch_size":1}))
            .expect("start vector reindex");
    assert_eq!(first["job_status"], "waiting");
    assert_eq!(first["progress"]["indexed_rows"], 1);
    assert!(first["checkpoint"]["source_digest"].is_string());
    assert!(first["checkpoint"].get("text").is_none());
    assert!(first["continuation"]["generation"].is_u64());

    let mut current = first;
    while !current["complete"].as_bool().unwrap_or(false) {
        current = super::do_resume_vector_reindex(&runtime, &current["continuation"])
            .expect("resume vector reindex");
    }
    assert_eq!(current["job_status"], "completed");
    assert_eq!(current["generation"], current["active_generation"]);
    let status = super::do_vector_reindex_status(
        &runtime,
        &json!({
            "namespace":"docs",
            "generation": current["generation"],
        }),
    )
    .expect("read vector reindex status");
    assert_eq!(status["generation_state"], "active");
    assert_eq!(status["progress"]["indexed_rows"], 3);

    assert!(
        super::do_start_vector_reindex(&runtime, &json!({"namespace":"docs", "batch_size":0}))
            .is_err()
    );
    assert!(super::do_resume_vector_reindex(
        &runtime,
        &json!({"namespace":"docs", "generation":0})
    )
    .is_err());
    let _ = fs::remove_dir_all(root);
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
    fs::write(root.join("c.md"), "gamma resumable document").expect("c fixture");
    let runtime = runtime(&root, "user:resume");

    let first = do_ingest(
        &runtime,
        &json!({
            "namespace": "docs",
            "paths": ["a.md", "b.md", "c.md"],
            "max_files_per_run": 1
        }),
    )
    .expect("start bounded ingest");
    assert_eq!(first["complete"], false);
    assert_eq!(first["job_status"], "waiting");
    assert_eq!(first["stats"]["job_processed_files"], 1);
    let job_id = first["job_id"].as_str().expect("job id").to_string();
    let checkpoint = super::storage::load_ingest_job(&runtime, &job_id).expect("checkpoint");
    assert!(super::do_resume_ingest(&runtime, &json!({"job_id": job_id})).is_err());

    let resumed =
        super::do_resume_ingest(&runtime, &first["continuation"]).expect("resume persisted ingest");
    assert_eq!(resumed["complete"], false);
    assert_eq!(resumed["job_status"], "waiting");
    assert_eq!(resumed["stats"]["job_processed_files"], 2);
    assert_eq!(resumed["stats"]["total_docs"], 2);

    let index = super::storage::load_namespace(&runtime, "docs").expect("index");
    let stale_commit =
        super::storage::save_namespace_and_job(&runtime, &index, &checkpoint, &checkpoint)
            .expect_err("an in-flight duplicate cannot overwrite a committed batch");
    assert!(stale_commit.is::<super::ingest::CheckpointConflict>());

    let stale = super::do_resume_ingest(&runtime, &first["continuation"])
        .expect_err("a repeated cursor must not advance another batch");
    let conflict = stale
        .downcast_ref::<super::ingest::CheckpointConflict>()
        .expect("typed checkpoint conflict");
    assert_eq!(conflict.extra()["error_code"], "checkpoint_conflict");
    assert_eq!(conflict.extra()["job"]["progress"]["processed_files"], 2);
    assert_eq!(
        conflict.extra()["job"]["continuation"],
        resumed["continuation"]
    );

    let completed = super::do_resume_ingest(&runtime, &resumed["continuation"])
        .expect("resume next persisted chunk");
    assert_eq!(completed["complete"], true);
    assert_eq!(completed["job_status"], "completed");
    assert_eq!(completed["stats"]["job_processed_files"], 3);
    assert_eq!(completed["stats"]["total_docs"], 3);

    let status = super::do_ingest_job_status(&runtime, &json!({"job_id": job_id}))
        .expect("load persisted job status");
    assert_eq!(status["job_status"], "completed");
    assert_eq!(status["progress"]["processed_files"], 3);
    let repeated = super::do_resume_ingest(&runtime, &json!({"job_id": job_id}))
        .expect("completed job is a safe no-op");
    assert_eq!(repeated, status);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn single_document_ingest_does_not_materialize_unrelated_chunk_bodies() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-row-level-ingest-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha original").expect("alpha fixture");
    fs::write(root.join("b.md"), "beta must remain untouched").expect("beta fixture");
    let runtime = runtime(&root, "user:row-level");
    let initial = do_ingest(
        &runtime,
        &json!({"namespace":"docs", "paths":["a.md", "b.md"]}),
    )
    .expect("initial ingestion");
    assert_eq!(initial["stats"]["total_docs"], 2);

    let db = rusqlite::Connection::open(&runtime.storage_database_path).expect("open KB database");
    let stable_retrieval_id: i64 = db
        .query_row(
            "SELECT i.id
             FROM memory_retrieval_index i
             JOIN kb_chunks c
               ON i.source_ref = 'kb:' || c.owner_user_key || ':' || c.namespace || ':' || c.chunk_id
             WHERE c.owner_user_key = ?1 AND c.namespace = 'docs'
               AND c.document_path = 'b.md'",
            [&runtime.scope_user_key],
            |row| row.get(0),
        )
        .expect("stable unrelated retrieval row");
    db.execute(
        "UPDATE kb_chunks SET text = CAST(X'FF' AS BLOB)
         WHERE owner_user_key = ?1 AND namespace = 'docs' AND document_path = 'b.md'",
        [&runtime.scope_user_key],
    )
    .expect("make unrelated body impossible to materialize as String");
    drop(db);

    fs::write(root.join("a.md"), "alpha changed").expect("change alpha fixture");
    let updated = do_ingest(&runtime, &json!({"namespace":"docs", "paths":["a.md"]}))
        .expect("row-level update must not read unrelated chunk bodies");
    assert_eq!(updated["stats"]["total_docs"], 2);
    assert_eq!(updated["stats"]["ingested_docs"], 1);

    let db =
        rusqlite::Connection::open(&runtime.storage_database_path).expect("reopen KB database");
    assert_eq!(
        db.query_row(
            "SELECT typeof(text) FROM kb_chunks
             WHERE owner_user_key = ?1 AND namespace = 'docs' AND document_path = 'b.md'",
            [&runtime.scope_user_key],
            |row| row.get::<_, String>(0),
        )
        .expect("unrelated body type"),
        "blob"
    );
    assert_eq!(
        db.query_row(
            "SELECT i.id
             FROM memory_retrieval_index i
             JOIN kb_chunks c
               ON i.source_ref = 'kb:' || c.owner_user_key || ':' || c.namespace || ':' || c.chunk_id
             WHERE c.owner_user_key = ?1 AND c.namespace = 'docs'
               AND c.document_path = 'b.md'",
            [&runtime.scope_user_key],
            |row| row.get::<_, i64>(0),
        )
        .expect("unrelated retrieval row after update"),
        stable_retrieval_id
    );
    drop(db);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bounded_overwrite_resets_once_and_resumes_with_row_level_appends() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-row-level-overwrite-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("old.md"), "old document").expect("old fixture");
    fs::write(root.join("new-a.md"), "new alpha").expect("alpha fixture");
    fs::write(root.join("new-b.md"), "new beta").expect("beta fixture");
    let runtime = runtime(&root, "user:overwrite");
    do_ingest(&runtime, &json!({"namespace":"docs", "paths":["old.md"]}))
        .expect("initial ingestion");

    let first = do_ingest(
        &runtime,
        &json!({
            "namespace":"docs",
            "paths":["new-a.md", "new-b.md"],
            "overwrite":true,
            "max_files_per_run":1
        }),
    )
    .expect("first overwrite batch");
    assert_eq!(first["complete"], false);
    assert_eq!(first["stats"]["total_docs"], 1);
    assert_eq!(
        super::storage::list_document_paths(&runtime, "docs").expect("paths after reset"),
        vec!["new-a.md".to_string()]
    );

    let completed =
        super::do_resume_ingest(&runtime, &first["continuation"]).expect("resume overwrite batch");
    assert_eq!(completed["complete"], true);
    assert_eq!(completed["stats"]["total_docs"], 2);
    assert_eq!(
        super::storage::list_document_paths(&runtime, "docs").expect("final paths"),
        vec!["new-a.md".to_string(), "new-b.md".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn chunk_transaction_budget_rewinds_next_file_into_continuation() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-chunk-budget-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    let body = "x".repeat(300);
    fs::write(root.join("a.md"), &body).expect("alpha fixture");
    fs::write(root.join("b.md"), &body).expect("beta fixture");
    let runtime = runtime(&root, "user:chunk-budget");

    let first = do_ingest(
        &runtime,
        &json!({
            "namespace":"docs",
            "paths":["a.md", "b.md"],
            "chunk_size":200,
            "chunk_overlap":0,
            "max_chunks_per_run":2
        }),
    )
    .expect("bounded first chunk batch");
    assert_eq!(first["complete"], false);
    assert_eq!(first["stats"]["processed_files"], 1);
    assert_eq!(first["stats"]["produced_chunks"], 2);
    assert_eq!(first["continuation"]["next_file_index"], 1);

    let completed = super::do_resume_ingest(&runtime, &first["continuation"])
        .expect("resume chunk-budget continuation");
    assert_eq!(completed["complete"], true);
    assert_eq!(completed["stats"]["processed_files"], 1);
    assert_eq!(completed["stats"]["job_processed_files"], 2);
    assert_eq!(completed["stats"]["total_docs"], 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persistent_document_quota_rolls_back_delta_and_marks_job_failed() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-document-quota-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha").expect("alpha fixture");
    fs::write(root.join("b.md"), "beta").expect("beta fixture");
    let runtime = runtime(&root, "user:quota");
    do_ingest(&runtime, &json!({"namespace":"docs", "paths":["a.md"]}))
        .expect("initial quota fixture");
    let db = rusqlite::Connection::open(&runtime.storage_database_path).expect("open quota db");
    db.execute(
        "UPDATE kb_storage_quotas SET max_documents = 1 WHERE owner_user_key = ?1",
        [&runtime.scope_user_key],
    )
    .expect("lower persistent document quota");
    drop(db);

    let error = do_ingest(&runtime, &json!({"namespace":"docs", "paths":["b.md"]}))
        .expect_err("second document must exceed quota");
    let policy = error
        .downcast_ref::<super::storage::StoragePolicyError>()
        .expect("typed storage policy error");
    assert_eq!(policy.extra()["error_code"], "storage_quota_exceeded");
    assert_eq!(policy.extra()["resource"], "documents");
    let db = rusqlite::Connection::open(&runtime.storage_database_path).expect("reopen quota db");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM kb_documents WHERE owner_user_key = ?1",
            [&runtime.scope_user_key],
            |row| row.get::<_, i64>(0),
        )
        .expect("document count after rejected delta"),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM kb_ingest_jobs
             WHERE owner_user_key = ?1 AND status = 'failed'",
            [&runtime.scope_user_key],
            |row| row.get::<_, i64>(0),
        )
        .expect("failed job count"),
        1
    );
    drop(db);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn all_persistent_storage_quotas_reject_writes_transactionally() {
    for (quota_column, resource, next_namespace) in [
        ("max_namespaces", "namespaces", "other"),
        ("max_documents", "documents", "docs"),
        ("max_chunks", "chunks", "docs"),
        ("max_vectors", "vectors", "docs"),
        ("max_database_bytes", "database_bytes", "docs"),
    ] {
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-kb-{resource}-quota-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("workspace");
        fs::write(root.join("a.md"), "alpha").expect("alpha fixture");
        fs::write(root.join("b.md"), "beta").expect("beta fixture");
        let runtime = runtime(&root, &format!("user:{resource}"));
        do_ingest(&runtime, &json!({"namespace":"docs", "paths":["a.md"]}))
            .expect("initial quota fixture");

        let db = rusqlite::Connection::open(&runtime.storage_database_path)
            .expect("open quota database");
        let before = [
            "kb_namespaces",
            "kb_documents",
            "kb_chunks",
            "kb_vector_projection",
        ]
        .map(|table| {
            db.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE owner_user_key = ?1"),
                [&runtime.scope_user_key],
                |row| row.get::<_, i64>(0),
            )
            .expect("count owner rows before rejected write")
        });
        let limit = match quota_column {
            "max_namespaces" => before[0],
            "max_documents" => before[1],
            "max_chunks" => before[2],
            "max_vectors" => before[3],
            "max_database_bytes" => 1,
            _ => unreachable!("fixed quota fixture"),
        };
        db.execute(
            &format!("UPDATE kb_storage_quotas SET {quota_column} = ?2 WHERE owner_user_key = ?1"),
            rusqlite::params![runtime.scope_user_key, limit],
        )
        .expect("lower persistent quota");
        drop(db);

        let error = do_ingest(
            &runtime,
            &json!({"namespace":next_namespace, "paths":["b.md"]}),
        )
        .expect_err("write beyond persistent quota must fail");
        let policy = error
            .downcast_ref::<super::storage::StoragePolicyError>()
            .expect("typed storage policy error");
        assert_eq!(policy.extra()["error_code"], "storage_quota_exceeded");
        assert_eq!(policy.extra()["resource"], resource);

        let db = rusqlite::Connection::open(&runtime.storage_database_path)
            .expect("reopen quota database");
        let after = [
            "kb_namespaces",
            "kb_documents",
            "kb_chunks",
            "kb_vector_projection",
        ]
        .map(|table| {
            db.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE owner_user_key = ?1"),
                [&runtime.scope_user_key],
                |row| row.get::<_, i64>(0),
            )
            .expect("count owner rows after rejected write")
        });
        assert_eq!(after, before, "{resource} quota must roll back all KB rows");
        drop(db);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn active_ingest_job_blocks_same_namespace_until_cancelled() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-ingest-concurrency-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha").expect("alpha fixture");
    fs::write(root.join("b.md"), "beta").expect("beta fixture");
    let runtime = runtime(&root, "user:concurrency");
    let waiting = do_ingest(
        &runtime,
        &json!({
            "namespace":"docs",
            "paths":["a.md", "b.md"],
            "max_files_per_run":1
        }),
    )
    .expect("create waiting ingestion");
    assert_eq!(waiting["job_status"], "waiting");

    let error = do_ingest(&runtime, &json!({"namespace":"docs", "paths":["b.md"]}))
        .expect_err("parallel namespace ingestion must be rejected");
    let policy = error
        .downcast_ref::<super::storage::StoragePolicyError>()
        .expect("typed concurrency error");
    assert_eq!(policy.extra()["error_code"], "storage_concurrency_limit");
    assert_eq!(policy.extra()["active_job_id"], waiting["job_id"].clone());

    super::do_cancel_ingest(
        &runtime,
        &json!({"job_id": waiting["job_id"].as_str().expect("job id")}),
    )
    .expect("cancel waiting ingestion");
    do_ingest(&runtime, &json!({"namespace":"docs", "paths":["b.md"]}))
        .expect("new ingestion after cancellation");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingestion_and_vector_reindex_share_namespace_and_owner_concurrency_limits() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-cross-job-concurrency-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("a.md"), "alpha").expect("alpha fixture");
    fs::write(root.join("b.md"), "beta").expect("beta fixture");
    let runtime = runtime(&root, "user:cross-concurrency");
    do_ingest(&runtime, &json!({"namespace":"other", "paths":["a.md"]}))
        .expect("create second namespace");
    let waiting = do_ingest(
        &runtime,
        &json!({
            "namespace":"docs",
            "paths":["a.md", "b.md"],
            "max_files_per_run":1
        }),
    )
    .expect("create waiting ingestion");
    let error = super::storage::start_vector_reindex(&runtime, "docs")
        .expect_err("same namespace reindex must wait for ingestion");
    let policy = error
        .downcast_ref::<super::storage::StoragePolicyError>()
        .expect("typed ingestion conflict");
    assert_eq!(policy.extra()["resource"], "namespace_active_ingest_jobs");
    super::do_cancel_ingest(
        &runtime,
        &json!({"job_id": waiting["job_id"].as_str().expect("job id")}),
    )
    .expect("cancel waiting ingestion");

    let vector = super::storage::start_vector_reindex(&runtime, "docs")
        .expect("start building vector generation");
    let error = do_ingest(&runtime, &json!({"namespace":"docs", "paths":["b.md"]}))
        .expect_err("same namespace ingestion must wait for vector reindex");
    let policy = error
        .downcast_ref::<super::storage::StoragePolicyError>()
        .expect("typed vector conflict");
    assert_eq!(
        policy.extra()["resource"],
        "namespace_active_vector_reindex_jobs"
    );

    let db = rusqlite::Connection::open(&runtime.storage_database_path)
        .expect("open concurrency database");
    db.execute(
        "UPDATE kb_storage_quotas SET max_active_jobs = 1 WHERE owner_user_key = ?1",
        [&runtime.scope_user_key],
    )
    .expect("lower shared active job quota");
    drop(db);
    let error = do_ingest(&runtime, &json!({"namespace":"other", "paths":["b.md"]}))
        .expect_err("vector and ingestion must share owner concurrency quota");
    let policy = error
        .downcast_ref::<super::storage::StoragePolicyError>()
        .expect("typed owner concurrency limit");
    assert_eq!(policy.extra()["resource"], "owner_active_kb_jobs");
    assert_eq!(policy.extra()["limit"], 1);

    super::storage::cancel_vector_reindex(&runtime, "docs", vector.generation)
        .expect("cancel vector generation");
    do_ingest(&runtime, &json!({"namespace":"other", "paths":["b.md"]}))
        .expect("ingestion after vector cancellation");
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
    let checkpoint = super::storage::load_ingest_job(&owner, job_id).expect("checkpoint");
    let index = super::storage::load_namespace(&owner, "docs").expect("index");
    let cancelled =
        super::do_cancel_ingest(&owner, &json!({"job_id": job_id})).expect("cancel owned job");
    assert_eq!(cancelled["job_status"], "cancelled");
    let stale = super::storage::save_namespace_and_job(&owner, &index, &checkpoint, &checkpoint)
        .expect_err("an in-flight batch cannot revive a cancelled job");
    assert!(stale.is::<super::ingest::CheckpointConflict>());
    assert_eq!(
        super::do_ingest_job_status(&owner, &json!({"job_id": job_id})).unwrap(),
        cancelled
    );
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
        storage_resource_profile: "standard".to_string(),
        storage_cache_size_kib: None,
        storage_mmap_size_bytes: None,
        storage_temp_store: None,
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
    assert!(matches!(
        search["stats"]["retrieval_mode"].as_str(),
        Some("fts5_candidates" | "hybrid_candidates")
    ));
    assert_eq!(search["stats"]["corpus_chunks"], 2);
    assert_eq!(search["stats"]["total_candidates"], 1);
    assert_eq!(search["hits"][0]["schema_version"], 2);
    assert_eq!(search["hits"][0]["instruction_authority"], "none");
    assert!(search["hits"][0]["source_ref"]
        .as_str()
        .is_some_and(|source_ref| source_ref.starts_with("kb://docs/")));
    assert!(search["hits"][0]["document_digest"]
        .as_str()
        .is_some_and(|digest| digest.len() == 64));
    let trace = &search["stats"]["retrieval_trace"];
    assert_eq!(trace["schema_version"], 1);
    assert!(trace["query_digest"]
        .as_str()
        .is_some_and(|digest| digest.len() == 64 && digest != "alpha"));
    assert_eq!(trace["filter_count"], 0);
    assert_eq!(trace["selected_hit_count"], 1);
    assert_eq!(
        trace["selected_hit_ids"][0],
        search["hits"][0]["evidence_id"]
    );
    assert_eq!(trace["no_answer"], false);
    assert!(trace["token_projection"]["token_budget"].as_u64().is_some());
    assert!(trace["latency_ms"]["total"].as_f64().is_some());
    assert!(trace["latency_ms"]["fts"].as_f64().is_some());
    assert!(trace["latency_ms"]["vector"].as_f64().is_some());
    assert!(trace["latency_ms"]["materialization"].as_f64().is_some());
    assert!(trace["latency_ms"]["rerank"].as_f64().is_some());
    assert!(!trace.to_string().contains("alpha document"));
    let removed_chunk_id = search["hits"][0]["chunk_id"]
        .as_str()
        .expect("removed document chunk id")
        .to_string();

    let removed = do_remove_documents(&runtime, &json!({"namespace": "docs", "paths": ["a.md"]}))
        .expect("remove document");
    assert_eq!(removed["removed_count"], 1);
    assert_eq!(removed["remaining_documents"], 1);
    let search_after_removal = do_search(
        &runtime,
        &json!({"namespace": "docs", "query": "alpha", "top_k": 2}),
    )
    .expect("search after document removal");
    assert_eq!(search_after_removal["result_kind"], "no_matching_evidence");
    assert_eq!(search_after_removal["hits"], json!([]));
    assert_eq!(
        search_after_removal["stats"]["retrieval_trace"]["no_answer"],
        true
    );
    {
        let db = rusqlite::Connection::open(&runtime.storage_database_path)
            .expect("open KB database after removal");
        let projection_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM kb_vector_projection
                 WHERE owner_user_key = ?1 AND namespace = 'docs' AND chunk_id = ?2",
                rusqlite::params![runtime.scope_user_key, removed_chunk_id],
                |row| row.get(0),
            )
            .expect("count removed vector rows");
        assert_eq!(projection_rows, 0);
    }

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
    {
        let db = rusqlite::Connection::open(&runtime.storage_database_path)
            .expect("open KB database after namespace deletion");
        let generations: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM kb_vector_generations
                 WHERE owner_user_key = ?1 AND namespace = 'docs'",
                rusqlite::params![runtime.scope_user_key],
                |row| row.get(0),
            )
            .expect("count deleted generations");
        let projections: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM kb_vector_projection
                 WHERE owner_user_key = ?1 AND namespace = 'docs'",
                rusqlite::params![runtime.scope_user_key],
                |row| row.get(0),
            )
            .expect("count deleted projections");
        assert_eq!(generations, 0);
        assert_eq!(projections, 0);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn document_listing_uses_stable_cursor_pagination() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-document-pagination-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("workspace");
    for name in ["a.md", "b.md", "c.md"] {
        fs::write(root.join(name), format!("content for {name}")).expect("document fixture");
    }
    let runtime = runtime(&root, "user:test");
    do_ingest(
        &runtime,
        &json!({
            "namespace": "docs",
            "paths": ["a.md", "b.md", "c.md"]
        }),
    )
    .expect("ingest documents");

    let first = do_list_documents(&runtime, &json!({"namespace": "docs", "limit": 2}))
        .expect("first document page");
    assert_eq!(first["document_count"], 3);
    assert_eq!(first["returned_count"], 2);
    assert_eq!(first["has_more"], true);
    assert_eq!(first["documents"][0]["path"], "a.md");
    assert_eq!(first["documents"][1]["path"], "b.md");
    assert_eq!(first["continuation"]["after_path"], "b.md");

    let second = do_list_documents(
        &runtime,
        &json!({
            "namespace": "docs",
            "limit": 2,
            "continuation": first["continuation"].clone()
        }),
    )
    .expect("second document page");
    assert_eq!(second["document_count"], 3);
    assert_eq!(second["returned_count"], 1);
    assert_eq!(second["has_more"], false);
    assert_eq!(second["documents"][0]["path"], "c.md");
    assert!(second["continuation"].is_null());

    let _ = fs::remove_dir_all(root);
}
