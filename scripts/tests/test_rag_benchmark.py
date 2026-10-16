#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "retrieval_validation" / "run_rag_benchmark.py"
SPEC = importlib.util.spec_from_file_location("run_rag_benchmark", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class RagBenchmarkTests(unittest.TestCase):
    def test_quality_metrics_separate_relevant_and_no_answer_cases(self) -> None:
        results = [
            MODULE.CaseResult(
                "first", 1.0, 1, True, False, True, ("a",), "exact", 4,
                citation_valid_count=1, citation_expected_count=1,
            ),
            MODULE.CaseResult(
                "second", 2.0, 2, True, False, True, ("x", "b"), "exact", 4,
                citation_valid_count=1, citation_expected_count=2,
            ),
            MODULE.CaseResult("miss", 3.0, None, True, False, False, ("x",), "exact", 4),
            MODULE.CaseResult("empty", 4.0, None, False, True, True, (), "exact", 4),
        ]

        metrics = MODULE.quality_metrics(results)

        self.assertEqual(metrics["case_count"], 4)
        self.assertEqual(metrics["recall_at_k"], 0.666667)
        self.assertEqual(metrics["mrr"], 0.5)
        self.assertEqual(metrics["ndcg"], 0.543643)
        self.assertEqual(metrics["citation_precision"], 0.666667)
        self.assertEqual(metrics["no_answer_accuracy"], 1.0)

    def test_database_inventory_reports_profiles_and_bytes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="rag-benchmark-test-") as raw:
            database = Path(raw) / "state.db"
            connection = sqlite3.connect(database)
            try:
                connection.executescript(
                    "CREATE TABLE kb_namespaces(id INTEGER);"
                    "CREATE TABLE kb_documents(id INTEGER);"
                    "CREATE TABLE kb_chunks(id INTEGER);"
                    "CREATE TABLE kb_vector_projection("
                    "owner_user_key TEXT, namespace TEXT, generation INTEGER, "
                    "profile_id TEXT, chunk_id TEXT);"
                    "CREATE TABLE kb_vector_ann_projection("
                    "owner_user_key TEXT, namespace TEXT, generation INTEGER, "
                    "profile_id TEXT, chunk_id TEXT, band INTEGER);"
                    "CREATE TABLE kb_embedding_profiles("
                    "profile_id TEXT, provider_kind TEXT, model_id TEXT, dimensions INTEGER, "
                    "normalization TEXT, profile_version TEXT, cross_language_semantic INTEGER);"
                    "CREATE TABLE kb_vector_generations("
                    "owner_user_key TEXT, namespace TEXT, generation INTEGER, profile_id TEXT, state TEXT);"
                    "CREATE TABLE memory_retrieval_index("
                    "embedding_model TEXT, embedding_dims INTEGER, embedding_version TEXT);"
                    "INSERT INTO kb_namespaces VALUES (1);"
                    "INSERT INTO kb_embedding_profiles VALUES "
                    "('local-hash-v3:24', 'local', 'local-hash-v3', 24, 'l2', 'local-hash-v3', 0);"
                    "INSERT INTO kb_vector_generations VALUES "
                    "('owner', 'namespace', 1, 'local-hash-v3:24', 'active');"
                    "INSERT INTO kb_vector_projection VALUES "
                    "('owner', 'namespace', 1, 'local-hash-v3:24', 'chunk-1');"
                    "INSERT INTO kb_vector_ann_projection VALUES "
                    "('owner', 'namespace', 1, 'local-hash-v3:24', 'chunk-1', 0);"
                    "INSERT INTO memory_retrieval_index VALUES ('local-hash-v3', 24, 'local-hash-v3');"
                )
                connection.commit()
            finally:
                connection.close()

            inventory = MODULE.database_inventory(database)

            self.assertEqual(inventory["row_counts"]["kb_namespaces"], 1)
            self.assertEqual(inventory["row_counts"]["kb_vector_ann_projection"], 1)
            self.assertEqual(inventory["active_vector_generations"], 1)
            self.assertEqual(inventory["active_embedding_profiles"][0]["version"], "local-hash-v3")
            self.assertGreater(inventory["database_bytes"], 0)

    def test_scale_gate_requires_complete_ann_projection_and_bounded_scoring(self) -> None:
        result = MODULE.CaseResult(
            "scale",
            1.0,
            1,
            True,
            False,
            True,
            ("target",),
            "ann_projection",
            4096,
        )
        inventory = {
            "row_counts": {
                "kb_chunks": 10_010,
                "kb_vector_projection": 10_010,
                "kb_vector_ann_projection": 10_010 * MODULE.ANN_BANDS,
            }
        }

        passed = MODULE.scale_gate(10_000, inventory, [result])
        self.assertTrue(passed["passed"])

        inventory["row_counts"]["kb_vector_ann_projection"] -= 1
        self.assertFalse(MODULE.scale_gate(10_000, inventory, [result])["passed"])

    def test_performance_gate_enforces_standard_scale_limits(self) -> None:
        passing = MODULE.performance_gate(
            100_000,
            {
                "search_latency_p95_ms": 499.0,
                "peak_child_rss_bytes": 99 * 1024 * 1024,
                "database_bytes_per_chunk": 7_000,
            },
        )
        self.assertTrue(passing["enforced"])
        self.assertTrue(passing["passed"])

        too_slow = MODULE.performance_gate(
            100_000,
            {
                "search_latency_p95_ms": 501.0,
                "peak_child_rss_bytes": 99 * 1024 * 1024,
                "database_bytes_per_chunk": 7_000,
            },
        )
        self.assertFalse(too_slow["passed"])
        too_large = MODULE.performance_gate(
            100_000,
            {
                "search_latency_p50_ms": 100.0,
                "search_latency_p95_ms": 200.0,
                "peak_child_rss_bytes": 50 * 1024 * 1024,
                "database_bytes_per_chunk": 9_000,
            },
        )
        self.assertFalse(too_large["passed"])
        self.assertFalse(MODULE.performance_gate(0, {})["enforced"])

    def test_threshold_gate_treats_injection_violations_as_maximum(self) -> None:
        thresholds = {
            "recall_at_k": 1.0,
            "ndcg": 1.0,
            "injection_violation_count": 0,
        }
        self.assertTrue(
            MODULE.thresholds_pass(
                {"recall_at_k": 1.0, "ndcg": 1.0, "injection_violation_count": 0},
                thresholds,
            )
        )
        self.assertFalse(
            MODULE.thresholds_pass(
                {"recall_at_k": 1.0, "ndcg": 1.0, "injection_violation_count": 1},
                thresholds,
            )
        )

    def test_ingest_continuation_runs_until_complete(self) -> None:
        initial = {
            "complete": False,
            "continuation": {"job_id": "job-1", "next_file_index": 2},
        }
        waiting = {
            "complete": False,
            "continuation": {"job_id": "job-1", "next_file_index": 4},
        }
        complete = {"complete": True, "status": "ok"}
        with mock.patch.object(MODULE, "invoke_skill", side_effect=[waiting, complete]) as invoke:
            final, slices = MODULE.resume_ingest_until_complete(
                Path("kb-skill"), Path("workspace"), initial
            )

        self.assertEqual(final, complete)
        self.assertEqual(slices, 3)
        self.assertEqual(invoke.call_args_list[0].args[3]["next_file_index"], 2)
        self.assertEqual(invoke.call_args_list[1].args[3]["next_file_index"], 4)

    def test_synthetic_scale_documents_are_bounded_shards(self) -> None:
        with tempfile.TemporaryDirectory(prefix="rag-scale-shards-") as raw:
            workspace = Path(raw)
            paths = MODULE.materialize_documents(
                workspace, {"documents": []}, MODULE.SYNTHETIC_ROWS_PER_DOCUMENT * 2 + 1
            )

            self.assertEqual(len(paths), 3)
            self.assertEqual(sum(path.read_text().count("\n") for path in paths), 10_001)

    def test_path_matching_is_platform_neutral(self) -> None:
        self.assertTrue(MODULE.path_matches("C:\\repo\\fixtures\\docs\\a.md", "fixtures/docs/a.md"))
        self.assertTrue(MODULE.path_matches("fixtures/docs/a.md", "fixtures/docs/a.md"))
        self.assertFalse(MODULE.path_matches("fixtures/docs/b.md", "fixtures/docs/a.md"))

    def test_generated_office_and_pdf_fixtures_are_real_packages(self) -> None:
        fixture = {
            "documents": [
                {"path": "docs/a.pdf", "generator": "pdf", "content": "PDF-1"},
                {"path": "docs/a.docx", "generator": "docx", "content": "DOCX-1"},
                {"path": "docs/a.xlsx", "generator": "xlsx", "content": "XLSX-1"},
                {"path": "docs/a.pptx", "generator": "pptx", "content": "PPTX-1"},
            ]
        }
        with tempfile.TemporaryDirectory(prefix="rag-format-fixtures-") as raw:
            paths = MODULE.materialize_documents(Path(raw), fixture, 0)
            self.assertEqual(paths[0].read_bytes()[:4], b"%PDF")
            for path in paths[1:]:
                self.assertEqual(path.read_bytes()[:2], b"PK")

    def test_citation_contract_requires_neutral_source_and_no_instruction_authority(self) -> None:
        hit = {
            "schema_version": 2,
            "source_ref": "kb://docs/document/chunk",
            "document_id": "document",
            "document_digest": "sha256:digest",
            "chunk_id": "chunk",
            "location": {"page": 1},
            "instruction_authority": "none",
        }
        self.assertTrue(MODULE.citation_is_valid(hit))
        hit["instruction_authority"] = "system"
        self.assertFalse(MODULE.citation_is_valid(hit))


if __name__ == "__main__":
    unittest.main()
