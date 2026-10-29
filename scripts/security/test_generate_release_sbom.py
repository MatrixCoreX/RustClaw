"""Verify lockfile inventory equivalence and pruning of excluded dependency trees."""
from pathlib import Path
import os
import tempfile
import unittest
from unittest import mock

from generate_release_sbom import IGNORED_PARTS, discover_lockfiles


class ReleaseSbomDiscoveryTests(unittest.TestCase):
    def test_pruned_scan_matches_existing_selection_without_visiting_ignored_trees(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = ["Cargo.lock", "UI/package-lock.json", "skills/example/requirements.txt"]
            for name in expected + [f"nested/{part}/deep/Cargo.lock" for part in IGNORED_PARTS]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            original_selection = sorted(path for path in root.rglob("*") if path.is_file()
                and not any(part in IGNORED_PARTS for part in path.relative_to(root).parts))
            scanned = []
            original_scandir = os.scandir

            def record(path):
                scanned.append(Path(path))
                return original_scandir(path)

            with mock.patch("os.scandir", side_effect=record):
                actual = discover_lockfiles(root)
            self.assertEqual(actual, original_selection)
            self.assertEqual([path.relative_to(root).as_posix() for path in actual], expected)
            self.assertFalse(any(part in IGNORED_PARTS for path in scanned for part in path.relative_to(root).parts))

    def test_directory_symlinks_are_not_traversed_and_order_is_deterministic(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source").mkdir()
            outside = root / "outside"
            outside.mkdir()
            (outside / "Cargo.lock").write_text("not-owned")
            (root / "source/external").symlink_to(outside, target_is_directory=True)
            for name in ["z/requirements.txt", "a/package-lock.json"]:
                path = root / "source" / name
                path.parent.mkdir()
                path.write_text("fixture")
            self.assertEqual([path.relative_to(root / "source").as_posix()
                for path in discover_lockfiles(root / "source")], ["a/package-lock.json", "z/requirements.txt"])


if __name__ == "__main__":
    unittest.main()
