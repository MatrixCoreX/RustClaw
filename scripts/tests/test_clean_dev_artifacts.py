"""Cache cleanup must never become a business-data retention command."""
import contextlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from scripts import clean_dev_artifacts as cleanup


@unittest.skipUnless(os.name == "posix", "cache mutation is supported on POSIX hosts only")
class CleanDevArtifactsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.addCleanup(mock.patch.stopall)
        mock.patch.object(cleanup, "ROOT", self.root).start()
        self.protected = (
            ".agent-runtime/operator.json", "configs/config.toml", "data/runtime.db",
            "logs/model_io.log.1", "logs/nl_tests/result.json", "tmp/acceptance/result.json",
            "scripts/nl_suite_logs/result.json", "UI/dist/index.html", "UI/node_modules/runtime.js",
        )
        for relative in self.protected:
            self.write(relative, b"owned-fixture\n")
        self.write("target/CACHEDIR.TAG", b"Signature: 8a477f597d28d172789f06886806bc55\n")
        self.write("target/debug/.cargo-lock", b"")
        self.cache = self.write("target/debug/incremental/unit/s-owned/dep-graph.bin", b"cache\n")
        self.binary = self.write("target/debug/runtime-fixture", b"runtime\n")
        self.binary.chmod(0o755)

    def write(self, relative, data):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def invoke(self, arguments):
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            result = cleanup.main(arguments)
        return result, output.getvalue()

    def assert_protected(self):
        for relative in self.protected:
            self.assertEqual((self.root / relative).read_bytes(), b"owned-fixture\n", relative)
        self.assertEqual(self.binary.read_bytes(), b"runtime\n")

    def test_apply_keeps_business_records_and_removes_real_compiler_cache(self):
        code, output = self.invoke(["--apply"])
        self.assertEqual(code, 0, output)
        self.assert_protected()
        self.assertFalse(self.cache.exists())

    def test_default_dry_run_does_not_select_business_directories(self):
        code, output = self.invoke([])
        self.assertEqual(code, 0, output)
        self.assertNotIn(".agent-runtime", output)
        self.assertNotIn("logs/", output)
        self.assertNotIn("tmp/", output)
        self.assertTrue(self.cache.exists())
        self.assert_protected()

    def test_business_log_and_dependency_deletion_flags_are_not_accepted(self):
        for option in ("--include-current-logs", "--include-node-modules"):
            with self.subTest(option=option), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    cleanup.parse_args([option])

    def test_apply_option_cannot_be_abbreviated(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            cleanup.parse_args(["--appl"])

    def test_runtime_libraries_executables_and_unknown_cache_files_remain(self):
        files = [self.write("target/debug/deps/" + name, b"keep\n") for name in (
            "libfixture.so", "libfixture.so.1", "libfixture.dylib", "fixture.dll",
            "credentials.json", "settings.toml", "private.key", "business.db", "model_io.log",
        )]
        executable = self.write("target/debug/deps/runtime-worker", b"keep\n")
        executable.chmod(0o755)
        files.append(executable)
        removable = [self.write("target/debug/deps/" + name, b"cache\n") for name in (
            "libfixture.rlib", "libfixture.rmeta", "fixture.d", "fixture.rcgu.o",
        )]
        self.assertEqual(self.invoke(["--apply"])[0], 0)
        for path in files:
            self.assertEqual(path.read_bytes(), b"keep\n")
        for path in removable:
            self.assertFalse(path.exists())
        self.assert_protected()

    def test_tag_is_required_and_invalid_tags_fail_before_any_deletion(self):
        tag = self.root / "target/CACHEDIR.TAG"
        for content in (None, b"", b"not-a-cargo-cache\n"):
            if content is None:
                tag.unlink()
            else:
                tag.write_bytes(content)
            self.assertEqual(self.invoke(["--apply"])[0], 2)
            self.assertTrue(self.cache.exists())
            self.assert_protected()

    def test_target_symlink_is_not_a_cleanup_grant(self):
        link = self.root / "target-link"
        link.symlink_to(self.root / "target", target_is_directory=True)
        code, _ = self.invoke(["--apply", "--target-dir", str(link)])
        self.assertEqual(code, 2)
        self.assertTrue(self.cache.exists())
        self.assert_protected()

    def test_profile_and_cache_symlinks_do_not_expose_other_directories(self):
        foreign = self.root / "foreign"
        foreign.mkdir()
        sentinel = foreign / "libprivate.rlib"
        sentinel.write_bytes(b"keep\n")
        (foreign / ".cargo-lock").write_bytes(b"")
        (self.root / "target/release").symlink_to(foreign, target_is_directory=True)
        (self.root / "target/debug/deps").symlink_to(foreign, target_is_directory=True)
        self.assertEqual(self.invoke(["--apply"])[0], 0)
        self.assertEqual(sentinel.read_bytes(), b"keep\n")
        self.assertTrue((self.root / "target/debug/deps").is_symlink())

    def test_descendant_directory_and_file_symlinks_are_not_followed(self):
        sentinel = self.write("outside/libprivate.rlib", b"keep\n")
        cache = self.root / "target/debug/incremental"
        (cache / "linked").symlink_to(sentinel.parent, target_is_directory=True)
        (cache / "linked.rlib").symlink_to(sentinel)
        self.assertEqual(self.invoke(["--apply"])[0], 0)
        self.assertEqual(sentinel.read_bytes(), b"keep\n")
        self.assertTrue((cache / "linked.rlib").is_symlink())

    def test_active_builder_denies_mutation(self):
        for builder in ("cargo", "rustc", "/usr/bin/clang"):
            with mock.patch.object(cleanup.subprocess, "check_output", return_value=builder + "\n"):
                code, output = self.invoke(["--apply"])
            self.assertEqual(code, 2, output)
            self.assertIn("cleanup_build_active", output)
            self.assertTrue(self.cache.exists())
            self.assert_protected()

    def test_busy_profile_lock_denies_mutation(self):
        child = subprocess.Popen([
            sys.executable, "-B", "-c",
            "import fcntl,sys; f=open(sys.argv[1],'rb'); "
            "fcntl.flock(f,fcntl.LOCK_EX); print('ready',flush=True); sys.stdin.read(1)",
            str(self.root / "target/debug/.cargo-lock"),
        ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), "ready")
            code, output = self.invoke(["--apply"])
            self.assertEqual(code, 2, output)
            self.assertTrue(self.cache.exists())
            self.assert_protected()
        finally:
            child.communicate(input="x", timeout=10)
        self.assertEqual(child.returncode, 0)

    def test_all_targets_are_admitted_before_any_file_is_unlinked(self):
        bad = self.root / "untagged-target"
        bad.mkdir()
        code, _ = self.invoke(["--apply", "--target-dir", str(self.root / "target"),
                               "--target-dir", str(bad)])
        self.assertEqual(code, 2)
        self.assertTrue(self.cache.exists())
        self.assert_protected()

    def test_duplicate_target_is_processed_only_once(self):
        target = str(self.root / "target")
        code, output = self.invoke(["--apply", "--target-dir", target, "--target-dir", target])
        self.assertEqual(code, 0, output)
        self.assertIn("files=1 ", output)
        self.assert_protected()

    def test_changed_file_is_preserved_instead_of_unlinked(self):
        with contextlib.ExitStack() as stack:
            files = cleanup.plan_cache_files(stack, [self.root / "target"])
            self.assertEqual(len(files), 1)
            self.cache.write_bytes(b"changed-current-payload\n")
            with self.assertRaisesRegex(RuntimeError, "cleanup_entry_changed"):
                cleanup.unlink_cache_file(files[0])
        self.assertEqual(self.cache.read_bytes(), b"changed-current-payload\n")

    def test_held_cache_descriptor_does_not_reopen_a_replaced_path(self):
        outside = self.write("outside/unit/s-owned/dep-graph.bin", b"keep\n")
        with contextlib.ExitStack() as stack:
            files = cleanup.plan_cache_files(stack, [self.root / "target"])
            cache = self.root / "target/debug/incremental"
            cache.rename(self.root / "admitted-cache")
            cache.symlink_to(outside.parents[2], target_is_directory=True)
            cleanup.unlink_cache_file(files[0])
        self.assertEqual(outside.read_bytes(), b"keep\n")
        self.assertFalse((self.root / "admitted-cache/unit/s-owned/dep-graph.bin").exists())

    def test_cross_target_profile_and_plain_profile_cache_are_both_supported(self):
        self.write("target/aarch64-unknown-linux-gnu/release/.cargo-lock", b"")
        cross = self.write("target/aarch64-unknown-linux-gnu/release/deps/fixture.rmeta", b"cache\n")
        self.assertEqual(self.invoke(["--apply"])[0], 0)
        self.assertFalse(cross.exists())
        self.assertFalse(self.cache.exists())

    def test_unsupported_platform_is_explicit_and_effect_free(self):
        with mock.patch.object(cleanup, "fcntl", None):
            code, output = self.invoke(["--apply"])
        self.assertEqual(code, 2)
        self.assertIn("cleanup_platform_unsupported", output)
        self.assertTrue(self.cache.exists())
        self.assert_protected()

    def test_fifo_marker_is_refused_without_waiting_for_a_writer(self):
        tag = self.root / "target/CACHEDIR.TAG"
        tag.unlink()
        os.mkfifo(tag)
        code, output = self.invoke(["--apply"])
        self.assertEqual(code, 2, output)
        self.assertTrue(self.cache.exists())
        self.assert_protected()

    def test_file_replaced_with_a_symlink_is_not_unlinked_or_followed(self):
        outside = self.write("outside/private.rlib", b"keep\n")
        with contextlib.ExitStack() as stack:
            files = cleanup.plan_cache_files(stack, [self.root / "target"])
            self.cache.unlink()
            self.cache.symlink_to(outside)
            with self.assertRaisesRegex(RuntimeError, "cleanup_entry_changed"):
                cleanup.unlink_cache_file(files[0])
        self.assertTrue(self.cache.is_symlink())
        self.assertEqual(outside.read_bytes(), b"keep\n")

    def test_explicit_unicode_target_and_cache_filenames_are_supported(self):
        target = self.root / "cache \u8d44\u6599 with spaces"
        target.mkdir()
        (target / "CACHEDIR.TAG").write_bytes(cleanup.CARGO_TAG + b"\n")
        profile = target / "debug"
        profile.mkdir()
        (profile / ".cargo-lock").write_bytes(b"")
        cache = profile / "deps"
        cache.mkdir()
        artifact = cache / "\u7f13\u5b58.o"
        artifact.write_bytes(b"cache\n")
        code, output = self.invoke(["--apply", "--target-dir", str(target)])
        self.assertEqual(code, 0, output)
        self.assertFalse(artifact.exists())
        self.assertTrue(self.cache.exists())
        self.assert_protected()


if __name__ == "__main__":
    unittest.main()
