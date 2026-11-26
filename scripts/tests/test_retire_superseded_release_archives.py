import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import retire_superseded_release_archives as retention


class RetentionTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.prior = Path(tmp.name) / "prior"
        self.successor = Path(tmp.name) / "successor"
        self.successor.mkdir()
        self.archives = []
        inventory = {"source":"a"*40, "files":[]}
        retention.save(self.successor/"state.json", {"repo":"test/repo", "base":"a"*40, "source":"b"*40})
        for platform, deploy, rollback in [("ubuntu","local-deployed","component-rollback"),
                                           ("pi","pi-deployed","pi-component-rollback")]:
            path = self.prior/"latest-packages"/("a"*12)/platform/"package.tar.gz"
            path.parent.mkdir(parents=True)
            path.write_bytes(platform.encode())
            self.archives.append(path)
            record = {"platform":platform,"name":path.name,"durable_path":str(path),
                      "bytes":path.stat().st_size,"sha256":retention.digest(path)}
            inventory["files"].append(record)
            saved = self.successor/(platform+".rollback")
            saved.write_bytes(b"rollback")
            retention.save(self.successor/(rollback+".json"),
                           {"source":"a"*40, "archive":str(saved), "archive_sha256":retention.digest(saved)})
            retention.save(self.successor/(deploy+".json"), {"source":"b"*40, "passed":True})
            for source, folder in [("a"*40,self.prior),("b"*40,self.successor)]:
                retention.save(folder/f"verified-{platform}.json",
                               {"platform":platform, "source":source, "files":[record],
                                "github_provenance_verified":True, "manifest_and_sbom_verified":True,
                                "signature_verified":True})
        retention.save(self.prior/"retained-release-assets.json",inventory)
        remote = patch.object(retention,"remote_artifact",return_value={"id":123})
        self.addCleanup(remote.stop)
        remote.start()
        handles = patch.object(retention.subprocess,"run")
        self.addCleanup(handles.stop)
        result = handles.start().return_value
        result.returncode, result.stdout, result.stderr = 1, b"", b""

    def run_tool(self, apply):
        with contextlib.redirect_stdout(io.StringIO()):
            retention.retire(self.prior,self.successor,apply)

    def test_dry_run_apply_and_retry(self):
        self.run_tool(False)
        self.assertTrue(all(p.exists() for p in self.archives))
        self.run_tool(True)
        self.assertFalse(any(p.exists() for p in self.archives))
        self.run_tool(True)
        self.assertTrue((self.successor/"ubuntu.rollback").is_file())
        self.assertTrue(json.loads((self.prior/"retained-release-assets.json").read_text())["full_archives_retired"])

    def test_one_bad_hash_preserves_both_packages(self):
        self.archives[-1].write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError,"local package mismatch"):
            self.run_tool(True)
        self.assertTrue(all(p.exists() for p in self.archives))

    def test_missing_archive_without_retirement_record(self):
        self.archives[-1].unlink()
        with self.assertRaisesRegex(ValueError,"unexplained missing"):
            self.run_tool(True)
        self.assertTrue(self.archives[0].exists())
