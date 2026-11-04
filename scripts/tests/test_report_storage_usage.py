"""All storage snapshots and writes use disposable fixtures, never runtime data."""
import importlib.util
import json
import os
import stat
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

SPEC = importlib.util.spec_from_file_location("report", Path(__file__).parents[1]/"report_storage_usage.py")
report = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(report)


class StorageReportTests(unittest.TestCase):
    def setUp(self):
        fixture = tempfile.TemporaryDirectory()
        self.addCleanup(fixture.cleanup)
        self.root = Path(fixture.name)

    def test_counts_files_without_reading_contents_and_preserves_data(self):
        data = self.root/"data"
        data.mkdir()
        secret = data/"credentials"
        secret.write_bytes(b"private fixture")
        with patch.object(Path, "read_bytes", side_effect=AssertionError("metadata only")):
            value = report.inventory(self.root)
        self.assertTrue(value["complete"])
        self.assertEqual(value["groups"]["data"]["apparent_bytes"], len(b"private fixture"))
        self.assertNotIn("credentials", json.dumps(value))
        self.assertEqual(secret.read_bytes(), b"private fixture")

    def test_skips_symlinks_and_deduplicates_hardlinks_across_groups(self):
        original = self.root/"original"
        original.write_bytes(b"same data")
        os.link(original, self.root/"hardlink")
        (self.root/"link").symlink_to(original)
        value = report.inventory(self.root)
        self.assertEqual(value["totals"]["unique_regular_files"], 1)
        self.assertEqual(value["totals"]["apparent_bytes"], 9)
        self.assertEqual(value["groups"]["link"]["symlinks_skipped"], 1)
        self.assertFalse(value["group_sizes_additive"])

    def test_sparse_files_report_actual_allocation_separately(self):
        path = self.root/"sparse"
        with path.open("wb") as stream:
            stream.seek(1024*1024)
            stream.write(b"x")
        row = report.inventory(self.root)["groups"]["sparse"]
        self.assertEqual(row["apparent_bytes"], 1024*1024+1)
        self.assertEqual(row["allocated_bytes"], path.stat().st_blocks*512)

    def test_compare_additions_changes_removals_and_scope(self):
        (self.root/"keep").write_bytes(b"one")
        (self.root/"gone").write_bytes(b"old")
        before = report.inventory(self.root)
        (self.root/"keep").write_bytes(b"one two")
        (self.root/"gone").unlink()
        (self.root/"new").write_bytes(b"x")
        after = report.inventory(self.root)
        changes = report.compare(after, before)["groups"]
        self.assertEqual(changes["keep"]["apparent_bytes_delta"], 4)
        self.assertEqual(changes["gone"]["apparent_bytes_delta"], -3)
        self.assertEqual(changes["new"]["apparent_bytes_delta"], 1)
        before["root_identity"]["inode"] += 1
        with self.assertRaisesRegex(ValueError, "scope_mismatch"):
            report.compare(after, before)

    def test_incomplete_or_unknown_counts_never_invent_zero_deltas(self):
        path = self.root/"file"
        path.write_bytes(b"fixture")
        before = report.inventory(self.root)
        after = report.inventory(self.root)
        before["groups"]["file"]["allocated_bytes"] = None
        self.assertIsNone(report.compare(after, before)["groups"]["file"]["allocated_bytes_delta"])
        before["groups"]["file"]["errors"] = 1
        self.assertIsNone(report.compare(after, before)["groups"]["file"]["apparent_bytes_delta"])
        del before["groups"]["file"]
        before["complete"] = False
        self.assertIsNone(report.compare(after, before)["groups"]["file"]["apparent_bytes_delta"])

    def test_report_output_is_private_and_never_overwrites_unrelated_files(self):
        value = report.inventory(self.root)
        output = self.root/"reports"/"snapshot.json"
        report.save_report(output, value)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        report.save_report(output, value)
        keep = self.root/"keep.json"
        keep.write_text('{"unrelated":"data"}')
        with self.assertRaises(ValueError):
            report.save_report(keep, value)
        self.assertEqual(keep.read_text(), '{"unrelated":"data"}')
        link = self.root/"link.json"
        link.symlink_to(keep)
        with self.assertRaises(ValueError):
            report.save_report(link, value)

    def test_invalid_baseline_values_are_rejected(self):
        value = report.inventory(self.root)
        value["groups"]["invalid"] = {"apparent_bytes": True, "allocated_bytes": 0, "errors": 0}
        with self.assertRaises(ValueError):
            report.validate_report(value)

    def test_mounts_and_special_files_are_not_traversed(self):
        boundary = self.root/"mounted"
        boundary.mkdir()
        (boundary/"outside").write_bytes(b"excluded")
        fifo = self.root/"fifo"
        os.mkfifo(fifo)
        original = Path.lstat
        with patch.object(Path, "lstat", lambda path: SimpleNamespace(st_dev=self.root.stat().st_dev+1,
                st_mode=stat.S_IFDIR) if path == boundary else original(path)):
            value = report.inventory(self.root)
        self.assertEqual(value["groups"]["mounted"]["mounts_skipped"], 1)
        self.assertEqual(value["groups"]["fifo"]["special_files_skipped"], 1)
        self.assertEqual(value["totals"]["apparent_bytes"], 0)

    def test_permission_errors_are_explicit_and_counts_are_lower_bounds(self):
        (self.root/"unreadable").write_bytes(b"private")
        with patch.object(Path, "lstat", side_effect=PermissionError):
            value = report.inventory(self.root)
        self.assertFalse(value["complete"])
        self.assertTrue(value["partial_counts_are_lower_bounds"])
        self.assertEqual(value["groups"]["unreadable"]["errors"], 1)

    def test_directory_alias_cycle_cannot_recurse_forever(self):
        alias = self.root/"alias"
        alias.mkdir()
        original = Path.lstat
        with patch.object(Path, "lstat", lambda path: self.root.stat() if path == alias else original(path)):
            value = report.inventory(self.root)
        self.assertEqual(value["groups"]["alias"]["directory_aliases_skipped"], 1)


if __name__ == "__main__":
    unittest.main()
