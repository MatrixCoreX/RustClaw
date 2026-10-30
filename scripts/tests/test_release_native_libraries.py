"""Cross-platform admission of optional chip libraries in release staging."""
from pathlib import Path
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from stage_release_pi_app import stage_app
from verify_release_binary import TARGETS, verify_binary


def native_header(target):
    kind, machine = TARGETS[target]
    header = bytearray(32)
    if kind == "elf":
        header[:6] = b"\x7fELF\x02\x01"
        header[18:20] = machine.to_bytes(2, "little")
    else:
        header[:4] = b"\xcf\xfa\xed\xfe"
        struct.pack_into("<I", header, 4, machine)
    return bytes(header)


class ReleaseNativeLibrariesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.library_root = self.source / "vendor/cryptoauthlib/python/cryptoauthlib"
        self.library_root.mkdir(parents=True)
        (self.library_root / "library.py").write_text("# loader fixture\n")
        (self.source / "signature.py").write_text("# signature fixture\n")
        self.destination = self.root / "package/pi_app"

    def library(self, name, target):
        path = self.library_root / name
        path.write_bytes(native_header(target))
        return path

    def test_all_native_targets_keep_only_their_matching_libraries(self):
        sources = {}
        for index, target in enumerate(TARGETS):
            suffix = f".so.{index}" if TARGETS[target][0] == "elf" else f".{index}.dylib"
            sources[target] = self.library("libcryptoauth" + suffix, target)
        for target in TARGETS:
            with self.subTest(target=target):
                destination = self.destination.parent / target
                result = stage_app(self.source, destination, target)
                self.assertEqual(result, {"target": target, "kept": 1, "excluded": 3})
                for source_target, source in sources.items():
                    staged = destination / source.relative_to(self.source)
                    self.assertEqual(staged.exists(), source_target == target)
                    if staged.exists():
                        self.assertEqual(staged.read_bytes(), source.read_bytes())
                        verify_binary(staged, target)
                self.assertTrue((destination / "signature.py").is_file())
                self.assertTrue((destination / self.library_root.relative_to(self.source) / "library.py").is_file())
                self.assertTrue(all(path.is_file() for path in sources.values()))

    def test_matching_aliases_become_regular_archive_files(self):
        for target in TARGETS:
            with self.subTest(target=target):
                suffix = ".so.3" if TARGETS[target][0] == "elf" else ".3.dylib"
                native = self.library("libcryptoauth" + suffix, target)
                alias = self.library_root / ("libcryptoauth.so" if suffix.startswith(".so") else "libcryptoauth.dylib")
                alias.symlink_to(native.name)
                native.chmod(0o755)
                destination = self.destination.parent / target
                self.assertEqual(stage_app(self.source, destination, target)["kept"], 2)
                staged_alias = destination / alias.relative_to(self.source)
                self.assertFalse(staged_alias.is_symlink())
                self.assertEqual(staged_alias.read_bytes(), native.read_bytes())
                self.assertEqual(staged_alias.stat().st_mode & 0o777, 0o755)
                self.assertTrue(alias.is_symlink())
                archive = self.root / "fixture.tar"
                with tarfile.open(archive, "w") as output:
                    output.add(destination, arcname="package")
                with tarfile.open(archive) as output:
                    self.assertFalse(any(item.issym() or item.islnk() for item in output))
                alias.unlink()
                native.unlink()

    def test_foreign_aliases_are_removed_without_modifying_source(self):
        native = self.library("libcryptoauth.so.3", "aarch64-unknown-linux-gnu")
        alias = self.library_root / "libcryptoauth.so"
        alias.symlink_to(native.name)
        result = stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
        self.assertEqual(result["kept"], 0)
        self.assertEqual(result["excluded"], 2)
        self.assertFalse(list(self.destination.rglob("libcryptoauth.so*")))
        self.assertTrue(native.is_file())
        self.assertTrue(alias.is_symlink())

    def test_malformed_or_unsupported_headers_fail_without_partial_destination(self):
        for invalid in (b"", b"#!/bin/sh\nexit 0\n", b"\x7fELF\x01\x01" + bytes(26), bytes(32)):
            with self.subTest(header=invalid):
                source = self.library_root / "libcryptoauth.so"
                source.write_bytes(invalid)
                self.library("libcryptoauth.so.3", "aarch64-unknown-linux-gnu")
                with self.assertRaisesRegex(ValueError, "release_chip_library_header_invalid"):
                    stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
                self.assertFalse(self.destination.exists())
                self.assertEqual(source.read_bytes(), invalid)
                self.assertFalse(list(self.destination.parent.glob(".release-pi-app-*")))

    def test_escaping_or_dangling_aliases_fail_without_reading_or_removing_target(self):
        external = self.root / "outside.so"
        external.write_bytes(native_header("x86_64-unknown-linux-gnu"))
        alias = self.library_root / "libcryptoauth.so"
        for target in (external, self.library_root / "missing.so"):
            with self.subTest(target=target.name):
                alias.symlink_to(target)
                with self.assertRaisesRegex(ValueError, "release_chip_library_path_invalid"):
                    stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
                self.assertFalse(self.destination.exists())
                self.assertTrue(alias.is_symlink())
                self.assertEqual(external.read_bytes(), native_header("x86_64-unknown-linux-gnu"))
                alias.unlink()

    def test_directory_aliases_and_library_directories_fail(self):
        alias = self.library_root / "nested"
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "release_chip_library_directory_alias_invalid"):
            stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
        alias.unlink()
        (self.library_root / "libcryptoauth.so").mkdir()
        with self.assertRaisesRegex(ValueError, "release_chip_library_path_invalid"):
            stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")

    def test_library_root_alias_is_rejected(self):
        root = self.source / "vendor/cryptoauthlib"
        shutil.rmtree(root)
        root.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "release_chip_library_root_invalid"):
            stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")

    def test_app_without_chip_library_remains_installable(self):
        shutil.rmtree(self.source / "vendor")
        result = stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
        self.assertEqual(result["kept"], 0)
        self.assertEqual(result["excluded"], 0)
        self.assertTrue((self.destination / "signature.py").is_file())

    def test_similarly_named_regular_files_are_not_classified_as_native_libraries(self):
        for name in ("libcryptoauth.software.py", "libcryptoauth_notes.dylib"):
            (self.library_root / name).write_text("fixture non-library content\n")
        result = stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
        self.assertEqual(result["kept"], 0)
        self.assertEqual(result["excluded"], 0)
        for name in ("libcryptoauth.software.py", "libcryptoauth_notes.dylib"):
            self.assertEqual(
                (self.destination / self.library_root.relative_to(self.source) / name).read_text(),
                "fixture non-library content\n",
            )

    def test_invalid_target_or_source_is_rejected_without_staging(self):
        with self.assertRaisesRegex(ValueError, "unsupported_release_target"):
            stage_app(self.source, self.destination, "unknown-target")
        with self.assertRaisesRegex(ValueError, "release_pi_app_source_invalid"):
            stage_app(self.root / "absent", self.destination, "x86_64-unknown-linux-gnu")
        source_alias = self.root / "source-alias"
        source_alias.symlink_to(self.source, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "release_pi_app_source_invalid"):
            stage_app(source_alias, self.destination, "x86_64-unknown-linux-gnu")

    def test_source_and_existing_destinations_are_not_modified(self):
        preserved = self.library("libcryptoauth.so", "aarch64-unknown-linux-gnu")
        for destination in (self.source, self.source / "nested", self.library_root):
            with self.assertRaisesRegex(ValueError, "release_pi_app_destination_invalid"):
                stage_app(self.source, destination, "x86_64-unknown-linux-gnu")
        self.destination.parent.mkdir()
        self.destination.symlink_to(self.root / "absent")
        with self.assertRaisesRegex(ValueError, "release_pi_app_destination_invalid"):
            stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
        self.assertEqual(preserved.read_bytes(), native_header("aarch64-unknown-linux-gnu"))

    def test_cli_emits_only_machine_staging_counts(self):
        self.library("libcryptoauth.so", "aarch64-unknown-linux-gnu")
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/stage_release_pi_app.py"), str(self.source),
             str(self.destination), "x86_64-unknown-linux-gnu"],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"target": "x86_64-unknown-linux-gnu", "kept": 0, "excluded": 1})

    def test_real_tracked_chip_libraries_are_preserved_only_for_the_matching_target(self):
        paths = subprocess.check_output(
            ["git", "-C", str(ROOT), "ls-files", "-z", "pi_app/vendor/cryptoauthlib"],
        ).decode().split("\0")
        headers = {}
        for relative in paths:
            if relative and Path(relative).name.startswith("libcryptoauth.so"):
                content = subprocess.check_output(["git", "-C", str(ROOT), "show", f"HEAD:{relative}"])
                destination = self.source / Path(relative).relative_to("pi_app")
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(content)
                headers[destination] = hashlib.sha256(content).hexdigest()
        self.assertTrue(headers)
        for target in TARGETS:
            destination = self.destination.parent / target
            stage_app(self.source, destination, target)
            for source, digest in headers.items():
                try:
                    verify_binary(source, target)
                    matches = True
                except ValueError:
                    matches = False
                staged = destination / source.relative_to(self.source)
                self.assertEqual(staged.is_file(), matches, f"{target}: {source.name}")
                self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), digest)
                if matches:
                    self.assertEqual(hashlib.sha256(staged.read_bytes()).hexdigest(), digest)

    def stage_signature_helper(self):
        paths = subprocess.check_output(
            ["git", "-C", str(ROOT), "ls-files", "-z", "pi_app/signature.py",
             "pi_app/signature_simulator.py", "pi_app/signature_simulator_x509.py",
             "pi_app/vendor/cryptoauthlib"],
        ).decode().split("\0")
        for relative in filter(None, paths):
            destination = self.source / Path(relative).relative_to("pi_app")
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(subprocess.check_output(["git", "-C", str(ROOT), "show", f"HEAD:{relative}"]))
        stage_app(self.source, self.destination, "x86_64-unknown-linux-gnu")
        return {
            "PATH": os.defpath, "HOME": str(self.root),
            "APP_SIGNATURE_SIMULATOR_STATE": str(self.root / "owned-simulator.json"),
            "APP_CRYPTOAUTHLIB_BOOTSTRAPPED": "1",
        }

    def test_staged_no_library_probe_retains_single_structured_error(self):
        environment = self.stage_signature_helper()
        wrapper = (
            "import os, runpy, sys\nfrom unittest.mock import patch\n"
            "sys.argv = sys.argv[1:]\nsys.path.insert(0, os.path.dirname(sys.argv[0]))\n"
            "with patch('ctypes.cdll.LoadLibrary', side_effect=OSError('fixture-library-unavailable')):\n"
            "    runpy.run_path(sys.argv[0], run_name='__main__')\n"
        )
        result = subprocess.run(
            [sys.executable, "-c", wrapper, str(self.destination / "signature.py"), "pubkey"],
            capture_output=True, text=True, env=environment, timeout=15,
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(len(result.stdout.splitlines()), 1)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["error_code"], "cryptoauthlib_unavailable")
        self.assertNotIn("Traceback", result.stderr)

    def test_staged_simulator_still_signs_without_any_native_library(self):
        environment = self.stage_signature_helper()
        results = []
        for arguments in (("simulation_enable",), ("pubkey",), ("sign_challenge", "ab" * 32)):
            result = subprocess.run(
                [sys.executable, str(self.destination / "signature.py"), *arguments],
                capture_output=True, text=True, env=environment, timeout=15,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(len(result.stdout.splitlines()), 1)
            self.assertNotIn("Traceback", result.stderr)
            payload = json.loads(result.stdout)
            self.assertTrue(payload["ok"])
            self.assertTrue(payload["simulated"])
            results.append(payload)
        self.assertEqual(results[0]["pubkey"], results[1]["pubkey"])
        self.assertEqual(len(results[2]["signature"]), 128)

    def test_all_platform_packagers_run_staging_before_archive_and_regression_tests(self):
        source = (ROOT / "package-release.sh").read_text()
        self.assertIn('"$SCRIPT_DIR/pi_app" "$STAGE_PROJECT_DIR/pi_app" "$APP_PACKAGE_TARGET"', source)
        self.assertLess(source.index("stage_release_pi_app.py"), source.index('tar -czf "$OUT"'))
        self.assertNotIn('copy_if_exists "pi_app"', source)
        for workflow in ("ubuntu-x86_64-release.yml", "pi-aarch64-release.yml", "macos-arm64-release-artifact.yml"):
            body = (ROOT / ".github/workflows" / workflow).read_text()
            self.assertIn("python3 -m unittest discover -s scripts/tests -p 'test_release*.py'", body)
            self.assertIn("./package-release.sh", body)


if __name__ == "__main__":
    unittest.main()
