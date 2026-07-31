#!/usr/bin/env python3
"""Tests for read-only existing WeChat artifact verification."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT_PATH = Path(__file__).with_name("verify_existing_wechat_artifact_delivery.py")


def load_verifier():
    spec = importlib.util.spec_from_file_location("wechat_artifact_verifier", SCRIPT_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ExistingWechatArtifactVerificationTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.verifier = load_verifier()

    def test_result_artifacts_requires_ui_download_and_preview_contract(self) -> None:
        valid = {
            "id": "artifact-1",
            "filename": "photo.webp",
            "kind": "image",
            "mime_type": "image/webp",
            "size_bytes": 3,
            "sha256": "a" * 64,
            "download_url": "/v1/tasks/task-1/artifacts/artifact-1/content",
            "preview_url": "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline",
        }
        invalid = {**valid, "preview_url": None}

        self.assertEqual(
            self.verifier.result_artifacts(json.dumps({"artifacts": [valid, invalid]})),
            [valid],
        )

    def test_execution_trace_requires_pinned_generation_and_receipts(self) -> None:
        binding = {
            "admission_receipt_digest": "a" * 64,
            "base_registry_digest": "b" * 64,
            "manifest_digest": "c" * 64,
            "overlay_generation_digest": "d" * 64,
            "policy_digest": "e" * 64,
            "receipt_digest": "f" * 64,
            "registry_generation": 45,
            "registry_generation_digest": "1" * 64,
            "skill_name": "media_download",
            "version": "0.1.0",
        }
        result = {
            "task_journal": {
                "trace": {
                    "rounds": [
                        {"first_action_capability_ref": "load_capability_groups"},
                        {"first_action_capability_ref": "media_download.download"},
                        {"first_action_capability_ref": "respond"},
                    ],
                    "capability_results": [
                        {
                            "capability": "media_download.download",
                            "action": "download",
                            "status": "ok",
                            "data": {
                                "extra": {
                                    "artifacts": [{"id": "artifact-1"}],
                                    "execution_binding": binding,
                                }
                            },
                        }
                    ],
                }
            }
        }

        summary = self.verifier.execution_trace_summary(json.dumps(result))

        self.assertEqual(
            summary["planner_actions"],
            ["load_capability_groups", "media_download.download", "respond"],
        )
        self.assertEqual(summary["execution_binding"], binding)
        del binding["receipt_digest"]
        self.assertIsNone(self.verifier.execution_trace_summary(json.dumps(result)))

    def test_logged_delivery_requires_upload_without_later_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "photo.webp"
            success = (
                "2026-07-31T00:00:00Z INFO wechat-ilink: outbound image uploaded "
                f"path={path} raw=3 cipher=16"
            )
            evidence = self.verifier.logged_wechat_delivery(success, path)
            self.assertIsNotNone(evidence)
            self.assertTrue(evidence["outbound_encrypted_upload_observed"])
            self.assertFalse(evidence["later_delivery_error_observed"])

            failed = success + f"\n2026-07-31T00:00:01Z WARN path={path} err=send_failed"
            failed_evidence = self.verifier.logged_wechat_delivery(failed, path)
            self.assertTrue(failed_evidence["later_delivery_error_observed"])

    def test_delivery_path_rejects_manifest_filename_traversal(self) -> None:
        artifact = {
            "id": "artifact-safe",
            "filename": "../../photo.webp",
        }

        with self.assertRaises(self.verifier.VerificationError):
            self.verifier.delivery_artifact_path("task-safe", artifact)


if __name__ == "__main__":
    unittest.main()
