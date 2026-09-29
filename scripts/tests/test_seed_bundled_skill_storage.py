#!/usr/bin/env python3

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/seed_bundled_skill_storage.py"


class SeedBundledSkillStorageTests(unittest.TestCase):
    def run_seed(self, source: Path, destination: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                "python3",
                str(SCRIPT),
                "--source",
                str(source),
                "--destination",
                str(destination),
            ],
            capture_output=True,
            text=True,
        )

    def test_initializes_missing_skill_and_preserves_existing_skill(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            destination = root / "destination"
            (source / "media_download/modelscope/model").mkdir(parents=True)
            (source / "media_download/modelscope/model/model.pt").write_bytes(b"model")
            (destination / "media_download").mkdir(parents=True)
            (destination / "media_download/user-data.txt").write_text("keep")
            (source / "media_discovery/cache").mkdir(parents=True)
            (source / "media_discovery/cache/seed.json").write_text("{}")

            result = self.run_seed(source, destination)
            self.assertEqual(result.returncode, 0, result.stderr)
            payload = json.loads(result.stdout)
            self.assertEqual(payload["skills"]["media_download"], "preserved_existing")
            self.assertEqual(payload["skills"]["media_discovery"], "initialized")
            self.assertEqual(
                (destination / "media_download/user-data.txt").read_text(), "keep"
            )
            self.assertFalse(
                (destination / "media_download/modelscope/model/model.pt").exists()
            )
            self.assertEqual(
                (destination / "media_discovery/cache/seed.json").read_text(), "{}"
            )

    def test_rejects_symlinked_seed_entries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            (source / "unsafe").symlink_to(root)
            result = self.run_seed(source, root / "destination")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unsupported seed entry", result.stderr)


if __name__ == "__main__":
    unittest.main()
