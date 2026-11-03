"""Cleanup tests use only disposable fixture roots, never the installed runtime."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("cleanup", Path(__file__).parents[1] / "clean_dev_artifacts.py")
cleanup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cleanup)


class CleanupTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="safe-cleanup-fixture-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name).resolve()
        self.addCleanup(patch.stopall)
        patch.object(cleanup, "ROOT", self.root).start()

    def file(self, name):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"fixture-data")
        return path

    def run_cleanup(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = cleanup.main(["--json", *args])
        return code, json.loads(out.getvalue())

    def test_preview_and_apply_preserve_all_runtime_data_and_installed_artifacts(self):
        protected = [self.file(p) for p in (
            ".agent-runtime/artifacts/delivery/video.mp4", ".agent-runtime/async_jobs/job.json",
            "data/agent-runtime.db", "data/skills/local_asr/model.pt", "logs/nl_tests/case.json",
            "scripts/nl_suite_logs/case.json", "tmp/evidence.txt", "UI/node_modules/installed.js",
            "UI/dist/index.html", "desktop/installers/setup.pkg", ".release-backups/old-clawd",
            "configs/config.toml", "target/release/clawd", "target/release/deps/test-binary",
            "target/release/deps/runtime.so", "target/skill-packages/receipt.json",
            "target/prebuilt-skill-packages/package/runtime.bin")]
        cached = [self.file(p) for p in (
            "target/release/.fingerprint/unit/info.json", "target/debug/incremental/unit/object.o",
            "target/release/deps/unit.rlib", "target/release/deps/unit.rmeta",
            "target/aarch64-unknown-linux-gnu/release/incremental/unit/object.o")]
        code, report = self.run_cleanup()
        self.assertEqual(code, 0)
        self.assertFalse(report["removed"])
        self.assertTrue(all(p.exists() for p in cached + protected))
        with patch.object(cleanup, "require_idle"):
            code, report = self.run_cleanup("--apply")
        self.assertEqual(code, 0)
        self.assertTrue(report["removed"])
        self.assertTrue(all(p.read_bytes() == b"fixture-data" for p in protected))
        self.assertTrue(all(not p.exists() for p in cached))

    def test_symlinked_cache_and_nested_links_are_rejected_without_deletion(self):
        for nested in (False, True):
            with self.subTest(nested=nested):
                target = self.root / "target/release/incremental"
                outside = self.root / ("outside-nested" if nested else "outside")
                outside.mkdir()
                keep = self.file(str(outside.relative_to(self.root) / "keep"))
                if nested:
                    target.mkdir()
                    (target / "link").symlink_to(outside, target_is_directory=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.symlink_to(outside, target_is_directory=True)
                code, _ = self.run_cleanup("--apply")
                self.assertEqual(code, 1)
                self.assertTrue(keep.exists())
                (target / "link").unlink() if nested else target.unlink()
                if nested:
                    target.rmdir()

    def test_busy_or_unavailable_audit_never_deletes(self):
        cached = self.file("target/debug/incremental/unit/object.o")
        with patch.object(cleanup, "require_idle", side_effect=RuntimeError("cleanup_cache_in_use")):
            code, report = self.run_cleanup("--apply")
        self.assertEqual(code, 1)
        self.assertEqual(report["error_code"], "cleanup_cache_in_use")
        self.assertTrue(cached.exists())

    def test_old_destructive_flags_are_not_accepted(self):
        for flag in ("--include-node-modules", "--include-current-logs"):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                cleanup.main([flag, "--apply"])

    def test_active_build_and_open_file_audits_fail_closed_on_linux_and_macos(self):
        path = self.file("target/debug/incremental/unit/object.o")
        for platform in ("linux", "darwin"):
            with self.subTest(platform=platform), patch.object(cleanup.sys, "platform", platform):
                with patch.object(cleanup.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "/usr/bin/cargo\n", "")):
                    with self.assertRaisesRegex(RuntimeError, "cleanup_build_active"):
                        cleanup.require_idle([path])
                for output, error, code in (("p123\nnfixture\n", "", 0), ("", "audit failed", 1), ("", "", 2)):
                    with patch.object(cleanup.shutil, "which", return_value="/usr/bin/lsof"), patch.object(cleanup.subprocess, "run", side_effect=[
                        subprocess.CompletedProcess([], 0, "runtime\n", ""),
                        subprocess.CompletedProcess([], code, output, error),
                    ]), self.assertRaises(RuntimeError):
                        cleanup.require_idle([path])
                with patch.object(cleanup.shutil, "which", return_value="/usr/bin/lsof"), patch.object(cleanup.subprocess, "run", side_effect=[
                    subprocess.CompletedProcess([], 0, "runtime\n", ""), subprocess.CompletedProcess([], 1, "", ""),
                ]):
                    cleanup.require_idle([path])
        self.assertTrue(path.exists())

    def test_only_known_unrelated_desktop_mount_warnings_can_be_ignored(self):
        path = self.file("target/debug/incremental/unit/object.o")
        known = "lsof: WARNING: can't stat() fuse.portal file system /run/user/1000/doc\n      Output information may be incomplete.\n"
        gvfs = known.replace("fuse.portal", "fuse.gvfsd-fuse").replace("/doc", "/gvfs")
        cases = [
            (known, path, True), (known + gvfs, path, True),
            (known.replace("/run/user/1000/doc", str(path.parent)), path, False),
            (known.replace("fuse.portal", "fuse.unknown"), path, False),
            (known.replace("/run/user/1000/doc", "/mnt/unaudited"), path, False),
            ("      Output information may be incomplete.\n", path, False),
            (known, Path("/run/user/1000/doc/cache"), False),
        ]
        for warning, target, allowed in cases:
            with self.subTest(warning=warning, target=target), patch.object(cleanup.sys, "platform", "linux"), \
                    patch.object(cleanup.shutil, "which", return_value="/usr/bin/lsof"), \
                    patch.object(cleanup.subprocess, "run", side_effect=[
                        subprocess.CompletedProcess([], 0, "runtime\n", ""),
                        subprocess.CompletedProcess([], 1, "", warning),
                    ]):
                if allowed:
                    cleanup.require_idle([target])
                else:
                    with self.assertRaisesRegex(RuntimeError, "cleanup_reference_audit_failed"):
                        cleanup.require_idle([target])
        self.assertTrue(path.exists())


if __name__ == "__main__":
    unittest.main()
