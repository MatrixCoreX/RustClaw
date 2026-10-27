#!/usr/bin/env python3
"""Release ratchet for the RAG storage, retrieval, and evidence boundaries."""

from __future__ import annotations

import argparse
import json
import re
import tempfile
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
KB_SCHEMA_VERSION = 7
ANN_PROJECTION_VERSION = "lsh-hyperplane-v2"
DOCUMENT_EXTRACTION_SCHEMA_VERSION = 1
RETRIEVAL_HIT_REQUIRED = {
    "schema_version",
    "evidence_id",
    "source_ref",
    "namespace",
    "namespace_revision",
    "document_id",
    "document_digest",
    "chunk_id",
    "chunk_ids",
    "location",
    "score",
    "retrieval_modes",
    "instruction_authority",
    "text",
}
EMBEDDING_LITERAL_OWNERS = {
    Path("crates/claw-core/src/retrieval_embedding.rs"): {
        "local-hash-v3": 3,
    },
    Path("crates/skills/kb/src/storage/database.rs"): {
        "local-hash-v3": 2,
    },
}
RAG_PRODUCTION_ROOTS = (
    Path("crates/claw-core/src"),
    Path("crates/clawd/src/memory"),
    Path("crates/skills/kb/src"),
)
REGISTRIES = (
    Path("configs/skills_registry.toml"),
    Path("docker/config/skills_registry.toml"),
)
KB_STORAGE_MODULES = ("database", "namespace_index", "retrieval", "vector_index")


def read_text(root: Path, relative: Path, findings: list[str]) -> str:
    try:
        return (root / relative).read_text(encoding="utf-8")
    except OSError:
        findings.append(f"missing_required_file:{relative.as_posix()}")
        return ""


def read_kb_storage(root: Path, findings: list[str]) -> str:
    facade = read_text(root, Path("crates/skills/kb/src/storage.rs"), findings)
    sources = [facade]
    for module in KB_STORAGE_MODULES:
        if f"mod {module};" not in facade:
            findings.append(f"kb_storage_module_not_wired:{module}")
        sources.append(
            read_text(root, Path(f"crates/skills/kb/src/storage/{module}.rs"), findings)
        )
    return "\n".join(sources)


def load_toml(root: Path, relative: Path, findings: list[str]) -> dict:
    text = read_text(root, relative, findings)
    if not text:
        return {}
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        findings.append(f"invalid_toml:{relative.as_posix()}:{exc}")
        return {}


def kb_registry_entry(raw: dict) -> dict:
    return next(
        (entry for entry in raw.get("skills", []) if entry.get("name") == "kb"),
        {},
    )


def validate_hit_schema(schema: dict, source: str, findings: list[str]) -> None:
    if schema.get("required") != ["text"]:
        findings.append(f"kb_business_output_envelope_drift:{source}")
    try:
        hit = schema["properties"]["extra"]["properties"]["hits"]["items"]
    except (KeyError, TypeError):
        findings.append(f"retrieval_hit_schema_missing:{source}")
        return
    required = set(hit.get("required", []))
    missing = sorted(RETRIEVAL_HIT_REQUIRED - required)
    if missing:
        findings.append(f"retrieval_hit_required_fields_missing:{source}:{','.join(missing)}")
    authority = hit.get("properties", {}).get("instruction_authority", {})
    if authority.get("const") != "none":
        findings.append(f"retrieval_instruction_authority_not_none:{source}")
    evidence = hit.get("properties", {}).get("evidence_id", {})
    if evidence.get("pattern") != "^evidence:sha256:[0-9a-f]{64}":
        findings.append(f"retrieval_evidence_id_pattern_drift:{source}")


def validate_kb_discovery_contract(
    entry: dict, capabilities_key: str, source: str, findings: list[str]
) -> None:
    if entry.get("group") != "memory" and capabilities_key == "planner_capabilities":
        findings.append(f"kb_registry_group_drift:{source}")
    if capabilities_key == "planner_capabilities" and entry.get("planner_eager_load") is not True:
        findings.append(f"kb_registry_eager_discovery_disabled:{source}")
    if capabilities_key == "planner_capabilities":
        queue = entry.get("dispatch_queue", {})
        required_actions = {
            "search",
            "ingest",
            "reindex",
            "resume_ingest",
            "start_vector_reindex",
            "resume_vector_reindex",
        }
        if queue.get("scope") != "user" or not required_actions.issubset(
            set(queue.get("actions", []))
        ):
            findings.append(f"kb_dispatch_queue_boundary_missing:{source}")
    capabilities = entry.get(capabilities_key, [])
    by_name = {
        item.get("name"): item for item in capabilities if isinstance(item, dict)
    }
    search = by_name.get("kb.search", {})
    required_tags = {
        "knowledge_base_search",
        "namespace_retrieval",
        "logical_namespace",
        "not_filesystem_path",
    }
    if not isinstance(search.get("description"), str) or not search["description"].strip():
        findings.append(f"kb_search_discovery_description_missing:{source}")
    if capabilities_key == "planner_capabilities" and not required_tags.issubset(
        set(search.get("semantic_tags", []))
    ):
        findings.append(f"kb_search_discovery_tags_missing:{source}")


def check_schema_parity(root: Path, findings: list[str]) -> None:
    for relative in REGISTRIES:
        entry = kb_registry_entry(load_toml(root, relative, findings))
        if not entry:
            findings.append(f"kb_registry_entry_missing:{relative.as_posix()}")
            continue
        storage = entry.get("storage", {})
        if storage != {
            "kind": "sqlite",
            "schema_version": KB_SCHEMA_VERSION,
            "migration_owner": "kb",
        }:
            findings.append(f"kb_storage_schema_drift:{relative.as_posix()}")
        validate_hit_schema(
            entry.get("output_schema", {}), relative.as_posix(), findings
        )
        validate_kb_discovery_contract(
            entry, "planner_capabilities", relative.as_posix(), findings
        )

    manifest_path = Path("crates/skills/kb/skill.toml")
    manifest = load_toml(root, manifest_path, findings)
    storage = manifest.get("storage", {})
    if storage != {
        "kind": "sqlite",
        "schema_version": KB_SCHEMA_VERSION,
        "migration_owner": "kb",
    }:
        findings.append("kb_manifest_storage_schema_drift")
    validate_hit_schema(
        manifest.get("capability_request", {}).get("output_schema", {}),
        manifest_path.as_posix(),
        findings,
    )
    validate_kb_discovery_contract(
        manifest.get("capability_request", {}),
        "capabilities",
        manifest_path.as_posix(),
        findings,
    )

    storage_source = read_text(root, Path("crates/skills/kb/src/storage.rs"), findings)
    if f"pub(super) const STORAGE_SCHEMA_VERSION: u32 = {KB_SCHEMA_VERSION};" not in storage_source:
        findings.append("kb_storage_implementation_schema_drift")


def check_installed_kb_package(root: Path, findings: list[str]) -> None:
    """Reject a locally activated KB package with an obsolete storage contract."""
    pointer_path = root / "data/skill-packages/kb/current.json"
    if not pointer_path.is_file():
        return
    try:
        pointer = json.loads(pointer_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        findings.append("kb_installed_package_pointer_invalid")
        return
    install_dir = pointer.get("install_dir")
    if not isinstance(install_dir, str) or not install_dir.strip():
        findings.append("kb_installed_package_pointer_invalid")
        return
    manifest_path = (
        Path("data/skill-packages/kb/versions") / install_dir / "skill.toml"
    )
    manifest = load_toml(root, manifest_path, findings)
    storage = manifest.get("storage", {})
    if storage != {
        "kind": "sqlite",
        "schema_version": KB_SCHEMA_VERSION,
        "migration_owner": "kb",
    }:
        findings.append("kb_installed_package_storage_schema_stale")


def rust_production_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for relative_root in RAG_PRODUCTION_ROOTS:
        absolute_root = root / relative_root
        if not absolute_root.exists():
            continue
        for path in absolute_root.rglob("*.rs"):
            relative = path.relative_to(root)
            if path.name.endswith("_tests.rs") or "tests" in relative.parts:
                continue
            files.append(relative)
    return sorted(set(files))


def check_embedding_literal_ownership(root: Path, findings: list[str]) -> None:
    literal_pattern = re.compile(r"local-hash-v[0-9]+")
    for relative in rust_production_files(root):
        text = read_text(root, relative, findings)
        values = literal_pattern.findall(text)
        allowed = EMBEDDING_LITERAL_OWNERS.get(relative)
        if values and allowed is None:
            findings.append(f"embedding_literal_outside_owner:{relative.as_posix()}")
            continue
        if allowed is None:
            continue
        actual = {value: values.count(value) for value in sorted(set(values))}
        if actual != allowed:
            findings.append(
                f"embedding_literal_owner_drift:{relative.as_posix()}:{actual!r}"
            )


def function_body(text: str, name: str) -> str:
    start = text.find(f"fn {name}(")
    if start < 0:
        return ""
    brace = text.find("{", start)
    if brace < 0:
        return ""
    depth = 0
    for index in range(brace, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[start : index + 1]
    return ""


def markers_precede(body: str, terminal: str, markers: tuple[str, ...]) -> bool:
    terminal_at = body.find(terminal)
    return terminal_at >= 0 and all(0 <= body.find(marker) < terminal_at for marker in markers)


def check_filter_before_limit(root: Path, findings: list[str]) -> None:
    storage = read_kb_storage(root, findings)
    fts = function_body(storage, "fts_candidate_chunk_ids")
    if not markers_precede(
        fts,
        "LIMIT ?10",
        ("AND (?5 IS NULL", "AND (?7 IS NULL", "AND (?8 IS NULL", "AND (?9 IS NULL"),
    ):
        findings.append("kb_fts_filter_before_limit_contract_missing")
    vector_filters = function_body(storage, "vector_filter_sql")
    filtered_rows = function_body(storage, "load_filtered_vector_rows")
    semantic = function_body(storage, "semantic_candidate_scores")
    filters_complete = all(
        marker in vector_filters
        for marker in (
            "c.document_path = ?",
            "c.file_type = ?",
            "c.mtime_epoch >= ?",
            "c.mtime_epoch <= ?",
        )
    )
    exact_filter_before_limit = markers_precede(
        filtered_rows, "{limit_clause}", ("vector_query_params", "{clauses}")
    )
    ann_filter_before_limit = markers_precede(
        semantic,
        "LIMIT ?20",
        ("c.document_path = ?7", "c.file_type = ?9", "c.mtime_epoch >= ?10", "c.mtime_epoch <= ?11"),
    )
    if not (filters_complete and exact_filter_before_limit and ann_filter_before_limit):
        findings.append("kb_semantic_filter_before_limit_contract_missing")


def check_runtime_boundaries(root: Path, findings: list[str]) -> None:
    generic_error_alias = re.compile(r'["\'](?:error_kind|code)["\']\s*:')
    authority_assignment = re.compile(r'instruction_authority\s*:\s*"([^"]+)"')
    authority_count = 0
    for relative in rust_production_files(root):
        text = read_text(root, relative, findings)
        if relative.parts[:4] == ("crates", "skills", "kb", "src"):
            if generic_error_alias.search(text):
                findings.append(f"kb_generic_error_alias:{relative.as_posix()}")
            for marker in ("agent-runtime.db", "database.sqlite_path"):
                if marker in text:
                    findings.append(
                        f"kb_main_storage_dependency:{relative.as_posix()}:{marker}"
                    )
        assignments = authority_assignment.findall(text)
        authority_count += len(assignments)
        if any(value != "none" for value in assignments):
            findings.append(f"retrieval_instruction_authority_assignment:{relative.as_posix()}")
    if authority_count < 2:
        findings.append("retrieval_instruction_authority_producers_missing")


def check_ann_projection_contract(root: Path, findings: list[str]) -> None:
    embedding = read_text(
        root, Path("crates/claw-core/src/retrieval_embedding.rs"), findings
    )
    storage = read_kb_storage(root, findings)
    recall = read_text(root, Path("crates/clawd/src/memory/retrieval.rs"), findings)
    host_schema = read_text(
        root, Path("crates/clawd/src/skill_storage/schema.rs"), findings
    )

    embedding_markers = (
        f'VECTOR_ANN_PROJECTION_VERSION: &str = "{ANN_PROJECTION_VERSION}"',
        "VECTOR_ANN_BANDS: usize = 8",
        "fn vector_ann_buckets(",
    )
    if any(marker not in embedding for marker in embedding_markers):
        findings.append("ann_embedding_projection_contract_missing")

    storage_markers = (
        "EXACT_VECTOR_LIMIT_STANDARD: usize = 4_096",
        "EXACT_VECTOR_LIMIT_CONSTRAINED: usize = 1_024",
        "ANN_MAX_CANDIDATES_STANDARD: usize = 4_096",
        "ANN_MAX_CANDIDATES_CONSTRAINED: usize = 1_024",
        "CREATE TABLE IF NOT EXISTS kb_vector_ann_projection",
        "fn verify_ann_projection(",
        'degradation: Some("ann_projection_stale".to_string())',
        '"ann_projection", rows',
        "content_kind TEXT NOT NULL DEFAULT 'plain_text'",
        "location_json TEXT NOT NULL DEFAULT '{}'",
    )
    if any(marker not in storage for marker in storage_markers):
        findings.append("kb_ann_projection_contract_missing")
    test_support = read_text(
        root, Path("crates/skills/kb/src/storage_support_tests.rs"), findings
    )
    if "fn rebuild_ann_projection(" not in test_support:
        findings.append("kb_ann_projection_test_support_missing")

    semantic = function_body(storage, "semantic_candidate_scores")
    exact_threshold_markers = (
        "generation_rows > exact_vector_limit.max(1)",
        "(!unfiltered).then_some(exact_vector_limit.max(1) + 1)",
        "rows.len() <= exact_vector_limit.max(1)",
        "ann_projection_available",
    )
    if not markers_precede(
        semantic, '("ann_projection", rows)', exact_threshold_markers
    ):
        findings.append("kb_exact_to_ann_threshold_contract_missing")

    recall_markers = (
        "KB_EXACT_VECTOR_LIMIT_STANDARD: usize = 4_096",
        "KB_EXACT_VECTOR_LIMIT_CONSTRAINED: usize = 1_024",
        "KB_ANN_MAX_CANDIDATES_STANDARD: usize = 4_096",
        "KB_ANN_MAX_CANDIDATES_CONSTRAINED: usize = 1_024",
        'sqlite_table_exists(db, "kb_vector_ann_projection")',
    )
    if any(marker not in recall for marker in recall_markers):
        findings.append("automatic_recall_ann_projection_contract_missing")

    host_markers = (
        f"KB_SCHEMA_VERSION: i64 = {KB_SCHEMA_VERSION}",
        "CREATE TABLE IF NOT EXISTS kb_vector_ann_projection",
        "schema_version = MAX(skill_storage_metadata.schema_version, excluded.schema_version)",
        "content_kind TEXT NOT NULL DEFAULT 'plain_text'",
        "location_json TEXT NOT NULL DEFAULT '{}'",
    )
    if any(marker not in host_schema for marker in host_markers):
        findings.append("kb_host_schema_ann_contract_missing")


def check_document_extraction_contract(root: Path, findings: list[str]) -> None:
    extraction = read_text(
        root, Path("crates/skills/kb/src/ingest_extract.rs"), findings
    )
    required_markers = (
        f"DOCUMENT_EXTRACTION_SCHEMA_VERSION: u32 = {DOCUMENT_EXTRACTION_SCHEMA_VERSION}",
        "struct DocumentExtractionResult",
        "struct DocumentTextBlock",
        "struct DocumentLocation",
        "struct ExtractionWarning",
        "schema_version: DOCUMENT_EXTRACTION_SCHEMA_VERSION",
        '"jsonl-records-v2"',
        '"html-visible-structure-v3"',
        "fn extract_visible_html_blocks(",
        "extract_pdf_text(path, None, None, limits)",
        "inspect_for_ingestion(path)",
        "MAX_SPREADSHEET_CELLS",
        "MAX_OFFICE_TABLE_ROWS",
        "fn extract_markdown_blocks(",
        "<script\\b[^>]*>.*?</script\\s*>",
        "<style\\b[^>]*>.*?</style\\s*>",
        'skip("binary_content_unsupported")',
        'skip("text_encoding_unsupported")',
        'skip("document_content_empty")',
    )
    if any(marker not in extraction for marker in required_markers):
        findings.append("kb_document_extraction_contract_missing")


def check_memory_runtime_startup_owner(root: Path, findings: list[str]) -> None:
    source = read_text(root, Path("crates/clawd/src/main.rs"), findings)
    load_at = source.find("let memory_runtime = load_memory_runtime_config")
    initialize_at = source.find("initialize_embedding_runtime(&db, &memory_runtime)")
    if load_at < 0 or initialize_at < 0 or load_at > initialize_at:
        findings.append("memory_runtime_config_loaded_after_embedding_initialization")
    required_markers = (
        "memory_runtime.background_job_concurrency",
        "memory_runtime.reindex_on_startup",
        "rebuild_retrieval_index(&db, &memory_runtime)",
    )
    if any(marker not in source for marker in required_markers):
        findings.append("memory_runtime_startup_config_not_authoritative")


def check_runtime_rebuild_safety(root: Path, findings: list[str]) -> None:
    source = read_text(root, Path("crates/clawd/src/memory/indexing.rs"), findings)
    rebuild = function_body(source, "rebuild_retrieval_index")
    transaction_at = rebuild.find("let tx = db.unchecked_transaction()?")
    delete_at = rebuild.find('tx.execute("DELETE FROM memory_retrieval_index"')
    commit_at = rebuild.rfind("tx.commit()?")
    if not (0 <= transaction_at < delete_at < commit_at):
        findings.append("memory_retrieval_rebuild_not_atomic")
    if "memory_retrieval_unowned_source_skipped" not in source:
        findings.append("memory_retrieval_unowned_legacy_boundary_missing")


def check_removed_profile_compatibility(root: Path, findings: list[str]) -> None:
    source = read_kb_storage(root, findings)
    forbidden = (
        "fn migrate_local_hash_v2_projection(",
        "fn migrate_legacy_vector_generations(",
        "fn migrate_legacy_namespace_projection(",
    )
    for marker in forbidden:
        if marker in source:
            findings.append(f"kb_retired_profile_compatibility_present:{marker[3:-1]}")
    cutover = function_body(source, "clear_inline_vector_payloads")
    if not all(
        marker in cutover
        for marker in (
            "vector_json = '[]'",
            "embedding_model = 'none'",
            "embedding_dims = 0",
            "embedding_version = 'none'",
        )
    ):
        findings.append("kb_inline_vector_cutover_missing")


def evaluate(root: Path) -> list[str]:
    findings: list[str] = []
    check_schema_parity(root, findings)
    check_installed_kb_package(root, findings)
    check_embedding_literal_ownership(root, findings)
    check_filter_before_limit(root, findings)
    check_runtime_boundaries(root, findings)
    check_ann_projection_contract(root, findings)
    check_document_extraction_contract(root, findings)
    check_memory_runtime_startup_owner(root, findings)
    check_runtime_rebuild_safety(root, findings)
    check_removed_profile_compatibility(root, findings)
    return findings


def fixture_hit_schema() -> str:
    required = ", ".join(f'"{field}"' for field in sorted(RETRIEVAL_HIT_REQUIRED))
    return (
        '{ type = "object", required = [' + required + '], properties = { '
        'instruction_authority = { type = "string", const = "none" }, '
        'evidence_id = { type = "string", pattern = "^evidence:sha256:[0-9a-f]{64}" } } }'
    )


def write_fixture(root: Path) -> None:
    hit = fixture_hit_schema()
    registry = (
        '[[skills]]\nname = "kb"\n'
        'group = "memory"\n'
        'planner_eager_load = true\n'
        'dispatch_queue = { scope = "user", actions = ["search", "ingest", "reindex", "resume_ingest", "start_vector_reindex", "resume_vector_reindex"] }\n'
        'storage = { kind = "sqlite", schema_version = 7, migration_owner = "kb" }\n'
        'planner_capabilities = [{ name = "kb.search", description = "fixture", semantic_tags = ["knowledge_base_search", "namespace_retrieval", "logical_namespace", "not_filesystem_path"] }]\n'
        f'output_schema = {{ required = ["text"], properties = {{ text = {{ type = "string" }}, extra = {{ properties = {{ hits = {{ items = {hit} }} }} }} }} }}\n'
    )
    manifest = (
        '[storage]\nkind = "sqlite"\nschema_version = 7\nmigration_owner = "kb"\n'
        '[capability_request]\n'
        'capabilities = [{ name = "kb.search", description = "fixture", semantic_tags = ["knowledge_base_search", "namespace_retrieval", "logical_namespace", "not_filesystem_path"] }]\n'
        f'output_schema = {{ required = ["text"], properties = {{ text = {{ type = "string" }}, extra = {{ properties = {{ hits = {{ items = {hit} }} }} }} }} }}\n'
    )
    storage = """pub(super) const STORAGE_SCHEMA_VERSION: u32 = 7;
const EXACT_VECTOR_LIMIT_STANDARD: usize = 4_096;
const EXACT_VECTOR_LIMIT_CONSTRAINED: usize = 1_024;
const ANN_MAX_CANDIDATES_STANDARD: usize = 4_096;
const ANN_MAX_CANDIDATES_CONSTRAINED: usize = 1_024;
fn fts_candidate_chunk_ids() { let _ = r#"AND (?5 IS NULL AND (?7 IS NULL AND (?8 IS NULL AND (?9 IS NULL LIMIT ?10"#; }
fn semantic_candidate_scores() {
    if generation_rows > exact_vector_limit.max(1) {}
    let _ = (!unfiltered).then_some(exact_vector_limit.max(1) + 1);
    let _ = rows.len() <= exact_vector_limit.max(1);
    ann_projection_available();
    let _ = r#"c.document_path = ?7 c.file_type = ?9 c.mtime_epoch >= ?10 c.mtime_epoch <= ?11 LIMIT ?20"#;
    let _ = ("ann_projection", rows);
    scored.truncate(1);
}
fn vector_filter_sql() {
    let _ = r#"c.document_path = ? c.file_type = ? c.mtime_epoch >= ? c.mtime_epoch <= ?"#;
}
fn load_filtered_vector_rows() {
    vector_query_params();
    let _ = r#"{clauses} ORDER BY c.chunk_id{limit_clause}"#;
    let limit_clause = " LIMIT ?";
}
fn rebuild_ann_projection() {}
fn verify_ann_projection() {}
fn ann_projection_available() {}
fn clear_inline_vector_payloads() {
    let _ = r#"vector_json = '[]' embedding_model = 'none' embedding_dims = 0 embedding_version = 'none'"#;
}
// CREATE TABLE IF NOT EXISTS kb_vector_ann_projection
// degradation: Some("ann_projection_stale".to_string())
// content_kind TEXT NOT NULL DEFAULT 'plain_text'
// location_json TEXT NOT NULL DEFAULT '{}'
// local-hash-v3 local-hash-v3
"""
    files = {
        "configs/skills_registry.toml": registry,
        "docker/config/skills_registry.toml": registry,
        "crates/skills/kb/skill.toml": manifest,
        "crates/claw-core/src/retrieval_embedding.rs": (
            'pub const VECTOR_ANN_PROJECTION_VERSION: &str = "lsh-hyperplane-v2";\n'
            "pub const VECTOR_ANN_BANDS: usize = 8;\n"
            "pub fn vector_ann_buckets() {}\n"
            "// local-hash-v3 local-hash-v3 local-hash-v3\n"
        ),
        "crates/clawd/src/memory/indexing.rs": (
            "fn rebuild_retrieval_index() {\n"
            "    let tx = db.unchecked_transaction()?;\n"
            "    tx.execute(\"DELETE FROM memory_retrieval_index\", [])?;\n"
            "    tx.commit()?;\n"
            "}\n"
            "// memory_retrieval_unowned_source_skipped\n"
        ),
        "crates/clawd/src/memory/retrieval.rs": (
            'const KB_EXACT_VECTOR_LIMIT_STANDARD: usize = 4_096;\n'
            'const KB_EXACT_VECTOR_LIMIT_CONSTRAINED: usize = 1_024;\n'
            'const KB_ANN_MAX_CANDIDATES_STANDARD: usize = 4_096;\n'
            'const KB_ANN_MAX_CANDIDATES_CONSTRAINED: usize = 1_024;\n'
            'fn hit() { let _ = Hit { instruction_authority: "none" }; '
            'sqlite_table_exists(db, "kb_vector_ann_projection"); }\n'
        ),
        "crates/skills/kb/src/main.rs": (
            'fn hit() { let _ = Hit { instruction_authority: "none" }; }\n'
        ),
        "crates/skills/kb/src/storage.rs": (
            storage[: storage.index("const EXACT_VECTOR_LIMIT_STANDARD")]
            + "\n".join(f"mod {module};" for module in KB_STORAGE_MODULES)
            + "\n"
        ),
        "crates/skills/kb/src/storage/database.rs": storage[
            storage.index("fn clear_inline_vector_payloads") :
        ],
        "crates/skills/kb/src/storage/namespace_index.rs": "// namespace fixture\n",
        "crates/skills/kb/src/storage/retrieval.rs": storage[
            storage.index("const EXACT_VECTOR_LIMIT_STANDARD") : storage.index(
                "fn rebuild_ann_projection"
            )
        ],
        "crates/skills/kb/src/storage/vector_index.rs": storage[
            storage.index("fn verify_ann_projection") : storage.index(
                "fn clear_inline_vector_payloads"
            )
        ],
        "crates/skills/kb/src/storage_support_tests.rs": "fn rebuild_ann_projection() {}\n",
        "crates/skills/kb/src/ingest_extract.rs": (
            "const DOCUMENT_EXTRACTION_SCHEMA_VERSION: u32 = 1;\n"
            "struct DocumentExtractionResult {}\n"
            "struct DocumentTextBlock {}\n"
            "struct DocumentLocation {}\n"
            "struct ExtractionWarning {}\n"
            "fn extract_document() { let _ = DOCUMENT_EXTRACTION_SCHEMA_VERSION; "
            "let _ = r#\"jsonl-records-v2 html-visible-structure-v3 "
            "<script\\b[^>]*>.*?</script\\s*> <style\\b[^>]*>.*?</style\\s*>\"#; "
            "let _ = (MAX_SPREADSHEET_CELLS, MAX_OFFICE_TABLE_ROWS); "
            "extract_pdf_text(path, None, None, limits); inspect_for_ingestion(path); "
            "let _ = (\"binary_content_unsupported\", \"text_encoding_unsupported\", "
            "\"document_content_empty\"); }\n"
            "fn extract_visible_html_blocks() {}\n"
            "fn extract_markdown_blocks() {}\n"
            "const MAX_SPREADSHEET_CELLS: usize = 1;\n"
            "const MAX_OFFICE_TABLE_ROWS: usize = 1;\n"
            "// schema_version: DOCUMENT_EXTRACTION_SCHEMA_VERSION\n"
            "// \"jsonl-records-v2\"\n"
            "// \"html-visible-structure-v3\"\n"
            "// skip(\"binary_content_unsupported\")\n"
            "// skip(\"text_encoding_unsupported\")\n"
            "// skip(\"document_content_empty\")\n"
        ),
        "crates/clawd/src/skill_storage/schema.rs": (
            "const KB_SCHEMA_VERSION: i64 = 7;\n"
            "// CREATE TABLE IF NOT EXISTS kb_vector_ann_projection\n"
            "// schema_version = MAX(skill_storage_metadata.schema_version, excluded.schema_version)\n"
            "// content_kind TEXT NOT NULL DEFAULT 'plain_text'\n"
            "// location_json TEXT NOT NULL DEFAULT '{}'\n"
        ),
        "crates/clawd/src/main.rs": (
            "fn run() {\n"
            "    let memory_runtime = load_memory_runtime_config(&workspace_root, &config.memory);\n"
            "    let _ = memory_runtime.background_job_concurrency;\n"
            "    initialize_embedding_runtime(&db, &memory_runtime);\n"
            "    if memory_runtime.reindex_on_startup {\n"
            "        rebuild_retrieval_index(&db, &memory_runtime);\n"
            "    }\n"
            "}\n"
        ),
    }
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="rag-contract-ratchet-") as raw:
        root = Path(raw)
        write_fixture(root)
        findings = evaluate(root)
        if findings:
            print(f"RAG_CONTRACT_SELF_TEST positive_failed={findings}")
            return 1
        retrieval = root / "crates/skills/kb/src/storage/retrieval.rs"
        retrieval.write_text(
            retrieval.read_text(encoding="utf-8").replace(
                "c.mtime_epoch >= ?10", "missing_time_filter"
            ),
            encoding="utf-8",
        )
        if "kb_semantic_filter_before_limit_contract_missing" not in evaluate(root):
            print("RAG_CONTRACT_SELF_TEST split_filter_negative_failed")
            return 1
        write_fixture(root)
        facade = root / "crates/skills/kb/src/storage.rs"
        facade.write_text(
            facade.read_text(encoding="utf-8").replace("mod retrieval;", ""),
            encoding="utf-8",
        )
        if "kb_storage_module_not_wired:retrieval" not in evaluate(root):
            print("RAG_CONTRACT_SELF_TEST split_wiring_negative_failed")
            return 1
        write_fixture(root)
        retrieval.unlink()
        if "missing_required_file:crates/skills/kb/src/storage/retrieval.rs" not in evaluate(root):
            print("RAG_CONTRACT_SELF_TEST split_missing_negative_failed")
            return 1
        write_fixture(root)
        main = root / "crates/skills/kb/src/main.rs"
        main.write_text(main.read_text(encoding="utf-8") + 'json!({"error_kind":"bad"});\n')
        findings = evaluate(root)
        if not any(item.startswith("kb_generic_error_alias:") for item in findings):
            print(f"RAG_CONTRACT_SELF_TEST negative_failed={findings}")
            return 1
        write_fixture(root)
        extraction = root / "crates/skills/kb/src/ingest_extract.rs"
        extraction.write_text(
            extraction.read_text(encoding="utf-8").replace(
                "DOCUMENT_EXTRACTION_SCHEMA_VERSION: u32 = 1",
                "DOCUMENT_EXTRACTION_SCHEMA_VERSION: u32 = 2",
            ),
            encoding="utf-8",
        )
        findings = evaluate(root)
        if "kb_document_extraction_contract_missing" not in findings:
            print(f"RAG_CONTRACT_SELF_TEST extraction_negative_failed={findings}")
            return 1
        write_fixture(root)
        startup = root / "crates/clawd/src/main.rs"
        startup.write_text(
            startup.read_text(encoding="utf-8").replace(
                "let memory_runtime = load_memory_runtime_config(&workspace_root, &config.memory);",
                "// memory runtime config was not loaded",
            ),
            encoding="utf-8",
        )
        findings = evaluate(root)
        if "memory_runtime_config_loaded_after_embedding_initialization" not in findings:
            print(f"RAG_CONTRACT_SELF_TEST startup_negative_failed={findings}")
            return 1
        write_fixture(root)
        indexing = root / "crates/clawd/src/memory/indexing.rs"
        indexing.write_text(
            indexing.read_text(encoding="utf-8").replace(
                'tx.execute("DELETE FROM memory_retrieval_index", [])?;',
                'db.execute("DELETE FROM memory_retrieval_index", [])?;',
            ),
            encoding="utf-8",
        )
        findings = evaluate(root)
        if "memory_retrieval_rebuild_not_atomic" not in findings:
            print(f"RAG_CONTRACT_SELF_TEST rebuild_negative_failed={findings}")
            return 1
        write_fixture(root)
        installed = root / "data/skill-packages/kb/versions/stale"
        installed.mkdir(parents=True)
        (installed / "skill.toml").write_text(
            "[storage]\nkind = \"sqlite\"\nschema_version = 3\n"
            "migration_owner = \"kb\"\n",
            encoding="utf-8",
        )
        pointer = root / "data/skill-packages/kb/current.json"
        pointer.parent.mkdir(parents=True, exist_ok=True)
        pointer.write_text('{"install_dir":"stale"}\n', encoding="utf-8")
        findings = evaluate(root)
        if "kb_installed_package_storage_schema_stale" not in findings:
            print(f"RAG_CONTRACT_SELF_TEST installed_negative_failed={findings}")
            return 1
        write_fixture(root)
        registry = root / "configs/skills_registry.toml"
        registry.write_text(
            registry.read_text(encoding="utf-8").replace(
                '"logical_namespace", "not_filesystem_path"',
                '"logical_namespace"',
            ),
            encoding="utf-8",
        )
        findings = evaluate(root)
        if not any(item.startswith("kb_search_discovery_tags_missing:") for item in findings):
            print(f"RAG_CONTRACT_SELF_TEST discovery_negative_failed={findings}")
            return 1
        write_fixture(root)
        storage = root / "crates/skills/kb/src/storage/database.rs"
        storage.write_text(
            storage.read_text(encoding="utf-8")
            + "\nfn migrate_legacy_vector_generations() {}\n",
            encoding="utf-8",
        )
        findings = evaluate(root)
        if not any(
            item.startswith("kb_retired_profile_compatibility_present:")
            for item in findings
        ):
            print(f"RAG_CONTRACT_SELF_TEST compatibility_negative_failed={findings}")
            return 1
    print("RAG_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    findings = evaluate(ROOT)
    if findings:
        print("RAG_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print("RAG_CONTRACT_CHECK ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
