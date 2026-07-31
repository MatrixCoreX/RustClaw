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
