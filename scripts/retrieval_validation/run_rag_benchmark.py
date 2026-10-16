#!/usr/bin/env python3
"""Run deterministic KB retrieval quality and storage baselines.

The benchmark never calls an LLM or a remote embedding provider. It creates an
isolated skill-owned database, invokes the KB JSONL process contract, computes
quality/latency/resource metrics, and removes the temporary workspace unless
explicitly retained.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import resource
import shutil
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_FIXTURE = ROOT / "scripts" / "fixtures" / "rag_retrieval" / "benchmark_v1.json"
DEFAULT_BINARY = ROOT / "target" / "debug" / "kb-skill"
TMP_ROOT = ROOT / "scripts" / "retrieval_validation" / ".tmp_runs"
USER_KEY = "rag-benchmark-principal"
SECONDARY_USER_KEY = "rag-benchmark-secondary-principal"
KB_SCHEMA_VERSION = 7
SYNTHETIC_ROWS_PER_DOCUMENT = 5_000
ANN_BANDS = 8
ANN_MAX_CANDIDATES_STANDARD = 4_096
STANDARD_SEARCH_P95_LIMIT_MS = 500.0
STANDARD_SEARCH_P50_LIMIT_MS = 300.0
STANDARD_PEAK_RSS_LIMIT_BYTES = 100 * 1024 * 1024
STANDARD_DATABASE_BYTES_PER_CHUNK_LIMIT = 8 * 1024


@dataclass(frozen=True)
class CaseResult:
    case_id: str
    latency_ms: float
    rank: int | None
    relevant: bool
    no_answer: bool
    passed: bool
    returned_paths: tuple[str, ...]
    vector_backend: str
    vector_rows_scored: int
    runtime_search_ms: float = 0.0
    candidate_generation_ms: float = 0.0
    fts_ms: float = 0.0
    vector_ms: float = 0.0
    materialization_ms: float = 0.0
    relevant_returned: int = 0
    returned_count: int = 0
    injection_violation_count: int = 0
    citation_valid_count: int = 0
    citation_expected_count: int = 0


def percentile(values: list[float], percentile_value: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int((len(ordered) - 1) * percentile_value + 0.999999)))
    return ordered[index]


def normalized_path(value: Any) -> str:
    return str(value or "").replace("\\", "/").lstrip("./")


def path_matches(actual: str, expected: str) -> bool:
    actual = normalized_path(actual)
    expected = normalized_path(expected)
    return actual == expected or actual.endswith(f"/{expected}")


def quality_metrics(results: list[CaseResult]) -> dict[str, float | int]:
    relevant = [result for result in results if result.relevant]
    no_answer = [result for result in results if result.no_answer]
    recall = sum(result.rank is not None for result in relevant) / max(1, len(relevant))
    mrr = sum(1.0 / result.rank for result in relevant if result.rank) / max(1, len(relevant))
    ndcg = sum(
        1.0 / math.log2(result.rank + 1) for result in relevant if result.rank
    ) / max(1, len(relevant))
    no_answer_accuracy = sum(result.passed for result in no_answer) / max(1, len(no_answer))
    returned_count = sum(result.returned_count for result in relevant)
    relevant_returned = sum(result.relevant_returned for result in relevant)
    citation_expected = sum(result.citation_expected_count for result in results)
    citation_valid = sum(result.citation_valid_count for result in results)
    return {
        "case_count": len(results),
        "relevant_case_count": len(relevant),
        "no_answer_case_count": len(no_answer),
        "recall_at_k": round(recall, 6),
        "mrr": round(mrr, 6),
        "ndcg": round(ndcg, 6),
        "retrieval_precision_in_context": round(
            relevant_returned / max(1, returned_count), 6
        ),
        "citation_precision": round(citation_valid / max(1, citation_expected), 6),
        "no_answer_accuracy": round(no_answer_accuracy, 6),
        "injection_violation_count": sum(
            result.injection_violation_count for result in results
        ),
    }


def current_child_peak_rss_bytes() -> int:
    peak = int(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)
    if sys.platform == "darwin":
        return peak
    return peak * 1024


def run_command(command: list[str], *, cwd: Path) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def ensure_binary(binary: Path, skip_build: bool) -> None:
    if not skip_build:
        run_command(["cargo", "build", "-p", "kb-skill"], cwd=ROOT)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"KB skill binary is unavailable: {binary}")


def invoke_skill(
    binary: Path,
    workspace: Path,
    request_id: str,
    args: dict[str, Any],
    *,
    user_key: str = USER_KEY,
) -> dict[str, Any]:
    database_path = workspace / "data" / "skills" / "kb" / "state.db"
    request = {
        "request_id": request_id,
        "args": args,
        "context": {
            "workspace_root": str(workspace),
            "user_key": user_key,
            "skill_storage": {
                "skill_name": "kb",
                "storage_kind": "sqlite",
                "schema_version": KB_SCHEMA_VERSION,
                "database_path": str(database_path),
                "database_busy_timeout_ms": 5000,
                "database_resource_profile": "standard",
                "database_cache_size_kib": 8192,
                "database_mmap_size_bytes": 0,
                "database_temp_store": "file",
            },
        },
        "user_id": 41002 if user_key == SECONDARY_USER_KEY else 41001,
        "chat_id": 42002 if user_key == SECONDARY_USER_KEY else 42001,
        "user_key": user_key,
    }
    completed = subprocess.run(
        [str(binary)],
        cwd=workspace,
        input=json.dumps(request, ensure_ascii=False) + "\n",
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"kb-skill exited with {completed.returncode}: {completed.stderr.strip()}"
        )
    records = []
    for line in completed.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as error:
            raise RuntimeError(f"kb-skill emitted non-JSON output: {line}") from error
    final = next(
        (record for record in reversed(records) if record.get("request_id") == request_id and record.get("status") in {"ok", "error"}),
        None,
    )
    if final is None:
        raise RuntimeError(f"kb-skill did not emit a final response for {request_id}")
    if final.get("status") != "ok":
        raise RuntimeError(
            f"kb-skill request {request_id} failed: {final.get('error_text') or final.get('extra')}"
        )
    extra = final.get("extra")
    if isinstance(extra, dict):
        return extra
    text = final.get("text")
    if isinstance(text, str) and text:
        parsed = json.loads(text)
        if isinstance(parsed, dict):
            return parsed
    raise RuntimeError(f"kb-skill request {request_id} has no structured payload")


def load_fixture(path: Path) -> dict[str, Any]:
    fixture = json.loads(path.read_text(encoding="utf-8"))
    if fixture.get("schema_version") != 1:
        raise RuntimeError("unsupported RAG benchmark fixture schema")
    if not isinstance(fixture.get("documents"), list) or not isinstance(fixture.get("cases"), list):
        raise RuntimeError("RAG benchmark fixture requires documents and cases arrays")
    return fixture


def write_minimal_pdf(path: Path, text: str) -> None:
    escaped = text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
    stream = f"BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET"
    objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        f"<< /Length {len(stream)} >>\nstream\n{stream}\nendstream",
    ]
    chunks = ["%PDF-1.4\n"]
    offsets: list[int] = []
    for index, obj in enumerate(objects, start=1):
        offsets.append(sum(len(chunk.encode("ascii")) for chunk in chunks))
        chunks.append(f"{index} 0 obj\n{obj}\nendobj\n")
    xref = sum(len(chunk.encode("ascii")) for chunk in chunks)
    chunks.extend([f"xref\n0 {len(objects) + 1}\n", "0000000000 65535 f \n"])
    chunks.extend(f"{offset:010} 00000 n \n" for offset in offsets)
    chunks.append(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    )
    path.write_bytes("".join(chunks).encode("ascii"))


def write_ooxml_package(path: Path, parts: dict[str, str]) -> None:
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(name, content)


def write_generated_document(path: Path, document: dict[str, Any]) -> None:
    generator = str(document.get("generator") or "")
    content = str(document.get("content") or "")
    if generator == "pdf":
        write_minimal_pdf(path, content)
        return
    if generator == "docx":
        write_ooxml_package(
            path,
            {
                "[Content_Types].xml": '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
                "_rels/.rels": '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
                "word/document.xml": f'<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>{document.get("heading", "Benchmark")}</w:t></w:r></w:p><w:p><w:r><w:t>{content}</w:t></w:r></w:p><w:sectPr/></w:body></w:document>',
            },
        )
        return
    if generator == "xlsx":
        sheet = str(document.get("sheet") or "Metrics")
        write_ooxml_package(
            path,
            {
                "[Content_Types].xml": '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
                "_rels/.rels": '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
                "xl/workbook.xml": f'<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{sheet}" sheetId="1" r:id="rId1"/></sheets></workbook>',
                "xl/_rels/workbook.xml.rels": '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
                "xl/worksheets/sheet1.xml": f'<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:B1"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>marker</t></is></c><c r="B1" t="inlineStr"><is><t>{content}</t></is></c></row></sheetData></worksheet>',
            },
        )
        return
    if generator == "pptx":
        write_ooxml_package(
            path,
            {
                "[Content_Types].xml": '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>',
                "_rels/.rels": '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>',
                "ppt/presentation.xml": '<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>',
                "ppt/_rels/presentation.xml.rels": '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>',
                "ppt/slides/slide1.xml": f'<?xml version="1.0"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/></p:nvSpPr><p:txBody><a:p><a:r><a:t>{document.get("title", "Benchmark")}</a:t></a:r></a:p><a:p><a:r><a:t>{content}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>',
            },
        )
        return
    raise RuntimeError(f"unsupported fixture generator: {generator}")


def materialize_documents(workspace: Path, fixture: dict[str, Any], synthetic_chunks: int) -> list[Path]:
    paths: list[Path] = []
    for document in fixture["documents"]:
        relative = Path(str(document["path"]))
        if relative.is_absolute() or ".." in relative.parts:
            raise RuntimeError(f"fixture path escapes workspace: {relative}")
        target = workspace / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        prefix = str(document.get("repeat_prefix") or "") * int(document.get("repeat_count") or 0)
        if document.get("generator"):
            if prefix:
                raise RuntimeError("generated fixture documents do not support repeat_prefix")
            write_generated_document(target, document)
        else:
            target.write_text(prefix + str(document.get("content") or ""), encoding="utf-8")
        paths.append(target)
    if synthetic_chunks > 0:
        scale_root = workspace / "fixtures" / "scale"
        scale_root.mkdir(parents=True, exist_ok=True)
        for shard_start in range(0, synthetic_chunks, SYNTHETIC_ROWS_PER_DOCUMENT):
            shard_end = min(synthetic_chunks, shard_start + SYNTHETIC_ROWS_PER_DOCUMENT)
            scale = scale_root / f"synthetic-scale-{shard_start:08d}.txt"
            with scale.open("w", encoding="utf-8") as handle:
                for index in range(shard_start, shard_end):
                    handle.write(
                        f"synthetic retrieval row {index:08d} scale-marker-{index:08d} "
                        + ("bounded benchmark filler " * 4)
                        + "\n"
                    )
            paths.append(scale)
    return paths


def database_inventory(database_path: Path) -> dict[str, Any]:
    if not database_path.is_file():
        raise RuntimeError(f"KB database does not exist: {database_path}")
    # Each benchmark skill process has exited before inventory starts, so the
    # database is an immutable snapshot. This also avoids macOS SQLite trying
    # to create WAL shared-memory files for a read-only connection.
    database_uri = f"{database_path.resolve().as_uri()}?mode=ro&immutable=1"
    connection = sqlite3.connect(database_uri, uri=True)
    try:
        table_names = {
            str(row[0])
            for row in connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        counts = {}
        for table in (
            "kb_namespaces",
            "kb_documents",
            "kb_chunks",
            "kb_vector_projection",
            "kb_vector_ann_projection",
            "memory_retrieval_index",
        ):
            counts[table] = (
                int(connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0])
                if table in table_names
                else None
            )
        legacy_profile_rows = (
            connection.execute(
                "SELECT embedding_model, embedding_dims, embedding_version, COUNT(*) "
                "FROM memory_retrieval_index GROUP BY embedding_model, embedding_dims, embedding_version "
                "ORDER BY embedding_model, embedding_dims, embedding_version"
            ).fetchall()
            if "memory_retrieval_index" in table_names
            else []
        )
        profile_rows = (
            connection.execute(
                "SELECT p.profile_id, p.provider_kind, p.model_id, p.dimensions, "
                "p.normalization, p.profile_version, p.cross_language_semantic, "
                "COUNT(DISTINCT CASE WHEN g.state = 'active' THEN "
                "g.owner_user_key || char(0) || g.namespace || char(0) || g.generation END), "
                "COUNT(CASE WHEN g.state = 'active' THEN v.chunk_id END) "
                "FROM kb_embedding_profiles p "
                "LEFT JOIN kb_vector_generations g ON g.profile_id = p.profile_id "
                "LEFT JOIN kb_vector_projection v "
                "ON v.owner_user_key = g.owner_user_key AND v.namespace = g.namespace "
                "AND v.generation = g.generation AND v.profile_id = p.profile_id "
                "GROUP BY p.profile_id, p.provider_kind, p.model_id, p.dimensions, "
                "p.normalization, p.profile_version, p.cross_language_semantic "
                "ORDER BY p.profile_id"
            ).fetchall()
            if {"kb_embedding_profiles", "kb_vector_generations", "kb_vector_projection"}
            <= table_names
            else []
        )
        active_generations = (
            int(
                connection.execute(
                    "SELECT COUNT(*) FROM kb_vector_generations WHERE state = 'active'"
                ).fetchone()[0]
            )
            if "kb_vector_generations" in table_names
            else 0
        )
        storage_schema_version = (
            connection.execute(
                "SELECT schema_version FROM skill_storage_metadata WHERE skill_name = 'kb'"
            ).fetchone()
            if "skill_storage_metadata" in table_names
            else None
        )
    finally:
        connection.close()
    related_bytes = sum(
        path.stat().st_size
        for path in (
            database_path,
            Path(f"{database_path}-wal"),
            Path(f"{database_path}-shm"),
        )
        if path.exists()
    )
    return {
        "database_path": str(database_path),
        "database_bytes": related_bytes,
        "storage_schema_version": (
            int(storage_schema_version[0]) if storage_schema_version is not None else None
        ),
        "generation_migration_required": not {
            "kb_embedding_profiles",
            "kb_vector_generations",
            "kb_vector_projection",
            "kb_vector_ann_projection",
        }
        <= table_names,
        "missing_tables": sorted(
            {
                "kb_embedding_profiles",
                "kb_vector_generations",
                "kb_vector_projection",
                "kb_vector_ann_projection",
            }
            - table_names
        ),
        "row_counts": counts,
        "active_vector_generations": active_generations,
        "active_embedding_profiles": [
            {
                "profile_id": row[0],
                "provider_kind": row[1],
                "model": row[2],
                "dimensions": row[3],
                "normalization": row[4],
                "version": row[5],
                "cross_language_semantic": bool(row[6]),
                "active_generation_count": row[7],
                "active_vector_row_count": row[8],
            }
            for row in profile_rows
        ],
        "legacy_retrieval_profiles": [
            {
                "model": row[0],
                "dimensions": row[1],
                "version": row[2],
                "row_count": row[3],
            }
            for row in legacy_profile_rows
        ],
    }


def citation_is_valid(hit: dict[str, Any]) -> bool:
    source_ref = hit.get("source_ref")
    location = hit.get("location")
    return (
        hit.get("schema_version") == 2
        and isinstance(source_ref, str)
        and source_ref.startswith("kb://")
        and isinstance(hit.get("document_id"), str)
        and bool(hit.get("document_id"))
        and isinstance(hit.get("document_digest"), str)
        and bool(hit.get("document_digest"))
        and isinstance(hit.get("chunk_id"), str)
        and bool(hit.get("chunk_id"))
        and isinstance(location, dict)
        and hit.get("instruction_authority") == "none"
    )


def location_matches(location: dict[str, Any], expected: dict[str, Any]) -> bool:
    for key in ("page", "sheet", "slide", "line_start", "line_end"):
        if key in expected and location.get(key) != expected[key]:
            return False
    section = expected.get("section_path_contains")
    if section is not None:
        actual = location.get("section_path")
        if not isinstance(actual, list) or section not in actual:
            return False
    return True


def run_case(binary: Path, workspace: Path, namespace: str, case: dict[str, Any]) -> CaseResult:
    args: dict[str, Any] = {
        "action": "search",
        "namespace": namespace,
        "query": str(case["query"]),
        "top_k": int(case.get("top_k") or 5),
    }
    for key in ("path_prefix", "file_type", "time_from", "time_to", "min_score"):
        if key in case:
            args[key] = case[key]
    started = time.perf_counter()
    user_key = (
        SECONDARY_USER_KEY if case.get("principal") == "secondary" else USER_KEY
    )
    payload = invoke_skill(
        binary,
        workspace,
        f"search-{case['id']}",
        args,
        user_key=user_key,
    )
    latency_ms = (time.perf_counter() - started) * 1000.0
    hits = payload.get("hits") if isinstance(payload.get("hits"), list) else []
    stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else {}
    vector_backend = str(stats.get("vector_candidate_backend") or "none")
    vector_rows_scored = max(0, int(stats.get("vector_rows_scored") or 0))
    retrieval_trace = (
        stats.get("retrieval_trace")
        if isinstance(stats.get("retrieval_trace"), dict)
        else {}
    )
    latency = (
        retrieval_trace.get("latency_ms")
        if isinstance(retrieval_trace.get("latency_ms"), dict)
        else {}
    )
    runtime_search_ms = float(latency.get("total") or 0.0)
    candidate_generation_ms = float(latency.get("candidate_generation") or 0.0)
    fts_ms = float(latency.get("fts") or 0.0)
    vector_ms = float(latency.get("vector") or 0.0)
    materialization_ms = float(latency.get("materialization") or 0.0)
    returned_paths = tuple(
        normalized_path(
            hit.get("path")
            or (hit.get("location") or {}).get("path")
            or (hit.get("metadata") or {}).get("path")
        )
        for hit in hits
        if isinstance(hit, dict)
    )
    citation_expected_count = len(hits)
    citation_valid_count = sum(
        citation_is_valid(hit) for hit in hits if isinstance(hit, dict)
    )
    expects_no_answer = bool(case.get("expect_no_answer"))
    if expects_no_answer:
        return CaseResult(
            case_id=str(case["id"]),
            latency_ms=latency_ms,
            rank=None,
            relevant=False,
            no_answer=True,
            passed=not hits,
            returned_paths=returned_paths,
            vector_backend=vector_backend,
            vector_rows_scored=vector_rows_scored,
            runtime_search_ms=runtime_search_ms,
            candidate_generation_ms=candidate_generation_ms,
            fts_ms=fts_ms,
            vector_ms=vector_ms,
            materialization_ms=materialization_ms,
            returned_count=len(returned_paths),
            citation_valid_count=citation_valid_count,
            citation_expected_count=citation_expected_count,
        )
    expected_paths = tuple(str(path) for path in case.get("expected_paths") or [])
    rank = next(
        (
            index
            for index, actual in enumerate(returned_paths, start=1)
            if any(path_matches(actual, expected) for expected in expected_paths)
        ),
        None,
    )
    relevant_returned = sum(
        any(path_matches(actual, expected) for expected in expected_paths)
        for actual in returned_paths
    )
    expected_location = case.get("expected_location")
    if rank is not None and isinstance(expected_location, dict):
        matching_hit = hits[rank - 1]
        if not isinstance(matching_hit, dict) or not location_matches(
            matching_hit.get("location") or {}, expected_location
        ):
            rank = None
    injection_violation_count = 0
    if bool(case.get("untrusted_content")):
        forbidden_fields = {"tool_call", "capability", "approval", "risk_level"}
        injection_violation_count = sum(
            hit.get("instruction_authority") != "none"
            or any(field in hit for field in forbidden_fields)
            for hit in hits
            if isinstance(hit, dict)
        )
    return CaseResult(
        case_id=str(case["id"]),
        latency_ms=latency_ms,
        rank=rank,
        relevant=True,
        no_answer=False,
        passed=rank is not None,
        returned_paths=returned_paths,
        vector_backend=vector_backend,
        vector_rows_scored=vector_rows_scored,
        runtime_search_ms=runtime_search_ms,
        candidate_generation_ms=candidate_generation_ms,
        fts_ms=fts_ms,
        vector_ms=vector_ms,
        materialization_ms=materialization_ms,
        relevant_returned=relevant_returned,
        returned_count=len(returned_paths),
        injection_violation_count=injection_violation_count,
        citation_valid_count=citation_valid_count,
        citation_expected_count=citation_expected_count,
    )


def resume_ingest_until_complete(
    binary: Path,
    workspace: Path,
    initial_payload: dict[str, Any],
    *,
    user_key: str = USER_KEY,
) -> tuple[dict[str, Any], int]:
    payload = initial_payload
    slices = 1
    while not bool(payload.get("complete")):
        continuation = payload.get("continuation")
        if not isinstance(continuation, dict):
            raise RuntimeError(f"incomplete ingest omitted continuation: {payload}")
        job_id = continuation.get("job_id")
        next_file_index = continuation.get("next_file_index")
        if not isinstance(job_id, str) or not job_id or not isinstance(next_file_index, int):
            raise RuntimeError(f"invalid ingest continuation: {continuation}")
        if slices >= 1_000:
            raise RuntimeError("ingest continuation exceeded 1000 bounded slices")
        payload = invoke_skill(
            binary,
            workspace,
            f"resume-ingest-{slices}",
            {
                "action": "resume_ingest",
                "job_id": job_id,
                "next_file_index": next_file_index,
            },
            user_key=user_key,
        )
        slices += 1
    return payload, slices


def resume_vector_reindex_until_complete(
    binary: Path,
    workspace: Path,
    namespace: str,
    initial_payload: dict[str, Any],
) -> tuple[dict[str, Any], int]:
    payload = initial_payload
    slices = 1
    while not bool(payload.get("complete")):
        continuation = payload.get("continuation")
        if not isinstance(continuation, dict):
            raise RuntimeError(f"incomplete vector reindex omitted continuation: {payload}")
        generation = continuation.get("generation")
        if not isinstance(generation, int) or generation <= 0:
            raise RuntimeError(f"invalid vector reindex continuation: {continuation}")
        if slices >= 1_000:
            raise RuntimeError("vector reindex exceeded 1000 bounded slices")
        payload = invoke_skill(
            binary,
            workspace,
            f"resume-vector-reindex-{slices}",
            {
                "action": "resume_vector_reindex",
                "namespace": namespace,
                "generation": generation,
                "batch_size": 32,
            },
        )
        slices += 1
    if payload.get("generation_state") != "active":
        raise RuntimeError(f"vector reindex did not activate: {payload}")
    return payload, slices


def run_lifecycle_cases(
    binary: Path,
    workspace: Path,
    namespace: str,
    lifecycle: dict[str, Any],
) -> tuple[list[CaseResult], dict[str, Any]]:
    relative = Path(str(lifecycle["path"]))
    path = workspace / relative
    path.write_text(str(lifecycle["updated_content"]), encoding="utf-8")
    ingest = invoke_skill(
        binary,
        workspace,
        "lifecycle-update",
        {
            "action": "ingest",
            "namespace": namespace,
            "paths": [str(path)],
            "overwrite": False,
        },
    )
    ingest, ingest_slices = resume_ingest_until_complete(binary, workspace, ingest)

    cases = [
        run_case(
            binary,
            workspace,
            namespace,
            {
                "id": "lifecycle_stale_content_absent",
                "query": lifecycle["stale_query"],
                "expect_no_answer": True,
                "top_k": 3,
                "min_score": 0.75,
            },
        ),
        run_case(
            binary,
            workspace,
            namespace,
            {
                "id": "lifecycle_updated_content_present",
                "query": lifecycle["updated_query"],
                "expected_paths": [str(relative)],
                "top_k": 3,
            },
        ),
    ]

    reindex = invoke_skill(
        binary,
        workspace,
        "lifecycle-start-vector-reindex",
        {
            "action": "start_vector_reindex",
            "namespace": namespace,
            "batch_size": 32,
        },
    )
    reindex, reindex_slices = resume_vector_reindex_until_complete(
        binary, workspace, namespace, reindex
    )
    cases.append(
        run_case(
            binary,
            workspace,
            namespace,
            {
                "id": "lifecycle_reindex_keeps_current_content",
                "query": lifecycle["updated_query"],
                "expected_paths": [str(relative)],
                "top_k": 3,
            },
        )
    )

    invoke_skill(
        binary,
        workspace,
        "lifecycle-remove-document",
        {
            "action": "remove_documents",
            "namespace": namespace,
            "paths": [str(path)],
        },
    )
    cases.append(
        run_case(
            binary,
            workspace,
            namespace,
            {
                "id": "lifecycle_deleted_content_absent",
                "query": lifecycle["updated_query"],
                "expect_no_answer": True,
                "top_k": 3,
                "min_score": 0.75,
            },
        )
    )
    return cases, {
        "ingest_slices": ingest_slices,
        "reindex_slices": reindex_slices,
        "activated_generation": reindex.get("generation"),
        "active_generation": reindex.get("active_generation"),
    }


def scale_gate(
    synthetic_chunks: int,
    inventory: dict[str, Any],
    results: list[CaseResult],
) -> dict[str, Any]:
    row_counts = inventory.get("row_counts") or {}
    actual_chunks = int(row_counts.get("kb_chunks") or 0)
    vector_rows = int(row_counts.get("kb_vector_projection") or 0)
    ann_rows = int(row_counts.get("kb_vector_ann_projection") or 0)
    backends = sorted({result.vector_backend for result in results})
    max_vector_rows_scored = max(
        (result.vector_rows_scored for result in results), default=0
    )
    if synthetic_chunks <= 0:
        passed = True
    else:
        passed = (
            actual_chunks >= synthetic_chunks
            and vector_rows >= synthetic_chunks
            and ann_rows == vector_rows * ANN_BANDS
            and "ann_projection" in backends
            and max_vector_rows_scored <= ANN_MAX_CANDIDATES_STANDARD
        )
    return {
        "requested_chunks": synthetic_chunks,
        "actual_chunks": actual_chunks,
        "vector_rows": vector_rows,
        "ann_rows": ann_rows,
        "expected_ann_rows": vector_rows * ANN_BANDS,
        "observed_vector_backends": backends,
        "max_vector_rows_scored": max_vector_rows_scored,
        "candidate_limit": ANN_MAX_CANDIDATES_STANDARD,
        "passed": passed,
    }


def thresholds_pass(metrics: dict[str, Any], thresholds: dict[str, Any]) -> bool:
    minimums_pass = all(
        float(metrics.get(key, 0.0)) >= float(thresholds.get(key, 0.0))
        for key in (
            "recall_at_k",
            "mrr",
            "ndcg",
            "retrieval_precision_in_context",
            "no_answer_accuracy",
            "citation_precision",
        )
        if key in thresholds
    )
    maximums_pass = all(
        float(metrics.get(key, float("inf"))) <= float(thresholds[key])
        for key in ("injection_violation_count",)
        if key in thresholds
    )
    return minimums_pass and maximums_pass


def performance_gate(synthetic_chunks: int, metrics: dict[str, Any]) -> dict[str, Any]:
    enforced = synthetic_chunks >= 10_000
    latency_p50 = float(metrics.get("search_latency_p50_ms") or 0.0)
    latency = float(metrics.get("search_latency_p95_ms") or 0.0)
    peak_rss = int(metrics.get("peak_child_rss_bytes") or 0)
    database_bytes_per_chunk = float(metrics.get("database_bytes_per_chunk") or 0.0)
    passed = not enforced or (
        latency_p50 <= STANDARD_SEARCH_P50_LIMIT_MS
        and latency <= STANDARD_SEARCH_P95_LIMIT_MS
        and peak_rss <= STANDARD_PEAK_RSS_LIMIT_BYTES
        and database_bytes_per_chunk <= STANDARD_DATABASE_BYTES_PER_CHUNK_LIMIT
    )
    return {
        "enforced": enforced,
        "search_latency_p50_ms": latency_p50,
        "search_latency_p50_limit_ms": STANDARD_SEARCH_P50_LIMIT_MS,
        "search_latency_p95_ms": latency,
        "search_latency_p95_limit_ms": STANDARD_SEARCH_P95_LIMIT_MS,
        "peak_child_rss_bytes": peak_rss,
        "peak_child_rss_limit_bytes": STANDARD_PEAK_RSS_LIMIT_BYTES,
        "database_bytes_per_chunk": round(database_bytes_per_chunk, 3),
        "database_bytes_per_chunk_limit": STANDARD_DATABASE_BYTES_PER_CHUNK_LIMIT,
        "passed": passed,
    }


def run_benchmark(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    fixture = load_fixture(args.fixture)
    ensure_binary(args.binary, args.skip_build)
    TMP_ROOT.mkdir(parents=True, exist_ok=True)
    workspace = Path(tempfile.mkdtemp(prefix="rag-benchmark-", dir=TMP_ROOT))
    namespace = "rag_benchmark_v1"
    try:
        documents = materialize_documents(workspace, fixture, args.synthetic_chunks)
        secondary_paths = {
            workspace / Path(str(document["path"]))
            for document in fixture["documents"]
            if document.get("principal") == "secondary"
        }
        primary_documents = [path for path in documents if path not in secondary_paths]
        ingest_started = time.perf_counter()
        ingest_payload = invoke_skill(
            args.binary,
            workspace,
            "ingest-benchmark",
            {
                "action": "ingest",
                "namespace": namespace,
                "paths": [str(path) for path in primary_documents],
                "overwrite": True,
                "chunk_size": 240,
                "chunk_overlap": 40,
                "max_file_size": max(2 * 1024 * 1024, args.synthetic_chunks * 256),
                "max_files_per_run": max(32, len(documents)),
                "max_bytes_per_run": max(64 * 1024 * 1024, args.synthetic_chunks * 320),
                "max_chunks_per_run": 10_000,
                "max_run_seconds": 300,
            },
        )
        ingest_payload, ingest_slices = resume_ingest_until_complete(
            args.binary, workspace, ingest_payload
        )
        ingest_latency_ms = (time.perf_counter() - ingest_started) * 1000.0
        if ingest_payload.get("status") != "ok":
            raise RuntimeError(f"benchmark ingest did not complete: {ingest_payload}")

        secondary_ingest_slices = 0
        if secondary_paths:
            secondary_ingest = invoke_skill(
                args.binary,
                workspace,
                "ingest-secondary-principal",
                {
                    "action": "ingest",
                    "namespace": namespace,
                    "paths": [str(path) for path in sorted(secondary_paths)],
                    "overwrite": True,
                },
                user_key=SECONDARY_USER_KEY,
            )
            secondary_ingest, secondary_ingest_slices = resume_ingest_until_complete(
                args.binary,
                workspace,
                secondary_ingest,
                user_key=SECONDARY_USER_KEY,
            )
            if secondary_ingest.get("status") != "ok":
                raise RuntimeError(
                    f"secondary benchmark ingest did not complete: {secondary_ingest}"
                )

        results = [run_case(args.binary, workspace, namespace, case) for case in fixture["cases"]]
        lifecycle_report: dict[str, Any] | None = None
        lifecycle = fixture.get("lifecycle")
        if isinstance(lifecycle, dict) and args.synthetic_chunks == 0:
            lifecycle_results, lifecycle_report = run_lifecycle_cases(
                args.binary, workspace, namespace, lifecycle
            )
            results.extend(lifecycle_results)
        metrics = quality_metrics(results)
        latencies = [result.latency_ms for result in results]
        metrics.update(
            {
                "ingest_latency_ms": round(ingest_latency_ms, 3),
                "ingest_slices": ingest_slices,
                "secondary_ingest_slices": secondary_ingest_slices,
                "search_latency_p50_ms": round(statistics.median(latencies), 3) if latencies else 0.0,
                "search_latency_p95_ms": round(percentile(latencies, 0.95), 3),
                "peak_child_rss_bytes": current_child_peak_rss_bytes(),
                "peak_rss_scope": (
                    "kb_skill_processes_only"
                    if args.skip_build
                    else "cargo_build_and_kb_skill_processes"
                ),
            }
        )
        database_path = workspace / "data" / "skills" / "kb" / "state.db"
        inventory = database_inventory(database_path)
        chunk_count = int((inventory.get("row_counts") or {}).get("kb_chunks") or 0)
        metrics["database_bytes"] = int(inventory.get("database_bytes") or 0)
        metrics["database_bytes_per_chunk"] = round(
            metrics["database_bytes"] / max(1, chunk_count), 3
        )
        scale = scale_gate(args.synthetic_chunks, inventory, results)
        performance = performance_gate(args.synthetic_chunks, metrics)
        report = {
            "schema_version": 1,
            "benchmark": fixture["name"],
            "fixture": str(args.fixture),
            "synthetic_chunks_requested": args.synthetic_chunks,
            "metrics": metrics,
            "thresholds": fixture.get("thresholds") or {},
            "passed": thresholds_pass(metrics, fixture.get("thresholds") or {})
            and bool(scale["passed"])
            and bool(performance["passed"]),
            "inventory": inventory,
            "scale_gate": scale,
            "performance_gate": performance,
            "lifecycle": lifecycle_report,
            "cases": [
                {
                    "id": result.case_id,
                    "passed": result.passed,
                    "rank": result.rank,
                    "latency_ms": round(result.latency_ms, 3),
                    "returned_paths": list(result.returned_paths),
                    "vector_backend": result.vector_backend,
                    "vector_rows_scored": result.vector_rows_scored,
                    "runtime_search_ms": round(result.runtime_search_ms, 3),
                    "process_overhead_ms": round(
                        max(0.0, result.latency_ms - result.runtime_search_ms), 3
                    ),
                    "candidate_generation_ms": round(
                        result.candidate_generation_ms, 3
                    ),
                    "fts_ms": round(result.fts_ms, 3),
                    "vector_ms": round(result.vector_ms, 3),
                    "materialization_ms": round(result.materialization_ms, 3),
                    "relevant_returned": result.relevant_returned,
                    "returned_count": result.returned_count,
                    "injection_violation_count": result.injection_violation_count,
                    "citation_valid_count": result.citation_valid_count,
                    "citation_expected_count": result.citation_expected_count,
                }
                for result in results
            ],
        }
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return report, 0 if report["passed"] else 1
    finally:
        if args.keep_temp:
            print(f"temporary_workspace={workspace}", file=sys.stderr)
        else:
            shutil.rmtree(workspace, ignore_errors=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, default=DEFAULT_FIXTURE)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--keep-temp", action="store_true")
    parser.add_argument("--synthetic-chunks", type=int, default=0)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--inventory-db", type=Path)
    parser.add_argument("--inventory-only", action="store_true")
    args = parser.parse_args()
    if args.synthetic_chunks < 0 or args.synthetic_chunks > 100_000:
        parser.error("--synthetic-chunks must be between 0 and 100000")
    if args.inventory_only and not args.inventory_db:
        parser.error("--inventory-only requires --inventory-db")
    return args


def main() -> int:
    args = parse_args()
    if args.inventory_only:
        report = {"schema_version": 1, "inventory": database_inventory(args.inventory_db)}
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0
    report, exit_code = run_benchmark(args)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
