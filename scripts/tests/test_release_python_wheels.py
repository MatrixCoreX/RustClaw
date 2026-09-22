import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_release_python_wheels import add_wheel_hashes, import_wheels, verify_wheels, wheel_identity


class ReleaseWheelTests(unittest.TestCase):
    def wheel(self, root, pure=True):
        path = root / "fixture_pkg-1.0-py3-none-any.whl"
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("fixture_pkg-1.0.dist-info/METADATA", "Name: fixture-pkg\nVersion: 1.0\n")
            archive.writestr("fixture_pkg-1.0.dist-info/WHEEL", f"Root-Is-Purelib: {str(pure).lower()}\n")
        return path

    def test_only_pure_source_builds_can_cross_platforms(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(wheel_identity(self.wheel(root), require_pure=True), ("fixture-pkg", "1.0"))
            with self.assertRaisesRegex(ValueError, "requires_native_build"):
                wheel_identity(self.wheel(root, pure=False), require_pure=True)

    def test_lock_preserves_pins_markers_and_original_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wheel = self.wheel(root)
            digest = hashlib.sha256(wheel.read_bytes()).hexdigest()
            lock = root / "requirements.lock"
            original = '--index-url https://pypi.org/simple\nfixture_pkg==1.0 ; sys_platform == "linux" \\\n    --hash=sha256:original\nother==2.0 --hash=sha256:unrelated\n'
            lock.write_text(original)
            add_wheel_hashes(lock, root)
            updated = lock.read_text()
            self.assertIn(f"--hash=sha256:{digest}", updated)
            self.assertIn('fixture_pkg==1.0 ; sys_platform == "linux"', updated)
            self.assertIn("--hash=sha256:original", updated)
            self.assertTrue(updated.endswith("other==2.0 --hash=sha256:unrelated\n"))
            add_wheel_hashes(lock, root)
            self.assertEqual(lock.read_text(), updated)

    def test_vendored_metadata_is_not_the_distribution_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            wheel = self.wheel(Path(directory))
            with zipfile.ZipFile(wheel, "a") as archive:
                archive.writestr("fixture/_vendor/nested.dist-info/METADATA", "Name: nested\nVersion: 9\n")
                archive.writestr("fixture/_vendor/nested.dist-info/WHEEL", "Root-Is-Purelib: true\n")
            self.assertEqual(wheel_identity(wheel), ("fixture-pkg", "1.0"))

    def test_install_verification_cannot_use_index_source_or_system_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.wheel(root)
            lock = root / "requirements.lock"
            lock.write_text("fixture-pkg==1.0 --hash=sha256:original\n")
            with patch("prepare_release_python_wheels.subprocess.run") as run:
                verify_wheels(lock, root)
            command = run.call_args.args[0]
            for option in ("--dry-run", "--target", "--ignore-installed", "--no-index",
                           "--only-binary=:all:", "--require-hashes", "--find-links"):
                self.assertIn(option, command)
            self.assertEqual(lock.read_text(), "fixture-pkg==1.0 --hash=sha256:original\n")

    def test_incomplete_wheel_and_truncated_lock_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wheel = root / "bad.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("missing", "data")
            with self.assertRaisesRegex(ValueError, "metadata_invalid"):
                wheel_identity(wheel)
            wheel.unlink()
            lock = root / "requirements.lock"
            lock.write_text("fixture==1.0 \\\n")
            with self.assertRaisesRegex(ValueError, "lock_truncated"):
                add_wheel_hashes(lock, root)

    def test_import_requires_both_abis_and_binds_target_source_and_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prepared = root / "prepared"
            relative = Path("optional_skills/fixture")
            staged = root / "stage" / relative
            staged.mkdir(parents=True)
            lock = staged / "requirements.lock"
            original = "fixture-pkg==1.0 --hash=sha256:original\n"
            lock.write_text(original)
            for version in ("3.13", "3.14"):
                package = prepared / version / relative
                wheels = package / "release-wheels"
                wheels.mkdir(parents=True)
                wheel = self.wheel(wheels)
                record = {"schema_version": 1, "target": "x86_64-unknown-linux-gnu",
                          "python_version": version,
                          "source_lock_sha256": hashlib.sha256(original.encode()).hexdigest(),
                          "wheels": [{"name": wheel.name, "sha256": hashlib.sha256(wheel.read_bytes()).hexdigest()}]}
                (package / "release-wheels.json").write_text(json.dumps(record))
                if version == "3.13":
                    with self.assertRaisesRegex(ValueError, "coverage_missing"):
                        import_wheels(lock, prepared, relative, record["target"])
            import_wheels(lock, prepared, relative, record["target"])
            selected = staged / "release-wheels" / wheel.name
            self.assertIn(hashlib.sha256(selected.read_bytes()).hexdigest(), lock.read_text())
            lock.write_text(original)
            with self.assertRaisesRegex(ValueError, "bundle_mismatch"):
                import_wheels(lock, prepared, relative, "aarch64-unknown-linux-gnu")
            wheel.write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "digest_mismatch"):
                import_wheels(lock, prepared, relative, record["target"])


if __name__ == "__main__":
    unittest.main()
