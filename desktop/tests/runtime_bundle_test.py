import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/prepare-macos-runtime-bundle.py"
SPEC = importlib.util.spec_from_file_location("runtime_bundle", SCRIPT)
runtime_bundle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime_bundle)


class RuntimeBundleTest(unittest.TestCase):
    def fixture(self, root: Path, target: str = "aarch64-apple-darwin"):
        source = root / "source"
        release = root / "release"
        output = root / "output"
        python_runtime = root / "python-runtime"
        for relative in runtime_bundle.BOOTSTRAP_FILES:
            path = source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture\n")
        (source / "configs/product_identity.toml").write_text(
            'schema_version=1\nrelease_artifact_id="sample-product"\n'
        )
        platform = runtime_bundle.platform_for_target(target)
        archive = release / f"sample-product-{platform}-1.2.3.tar.gz"
        release.mkdir()
        archive.write_bytes(b"archive")
        Path(f"{archive}.sha256").write_text("0" * 64 + "  " + archive.name)
        Path(f"{archive}.spdx.json").write_text("{}")
        Path(f"{archive}.manifest.json").write_text(json.dumps({
            "release": {"target": target, "package_root": "sample-product"}
        }))
        Path(f"{archive}.manifest.json.sig").write_text("signature")
        python_bin = python_runtime / "bin/python3"
        python_bin.parent.mkdir(parents=True)
        python_bin.write_text("#!/bin/sh\nexit 0\n")
        python_bin.chmod(0o755)
        return source, release, output, platform, python_runtime

    def test_stages_signed_release_and_neutral_bootstrap(self):
        with tempfile.TemporaryDirectory() as directory:
            source, release, output, platform, python_runtime = self.fixture(Path(directory))
            descriptor = runtime_bundle.stage(
                source,
                release,
                output,
                "aarch64-apple-darwin",
                f"{platform}-1.2.3",
                python_runtime,
            )
            self.assertEqual(descriptor["schema_version"], 1)
            self.assertEqual(descriptor["platform"], "macos-aarch64")
            self.assertTrue((output / "release" / descriptor["archive"]).is_file())
            self.assertTrue((output / "bootstrap/deploy-github-release.sh").is_file())
            self.assertTrue((output / "bootstrap/toolchains/python/bin/python3").is_file())
            self.assertEqual(json.loads((output / "bundle.json").read_text()), descriptor)

    def test_rejects_wrong_target_tag_and_missing_companion(self):
        with tempfile.TemporaryDirectory() as directory:
            source, release, output, platform, python_runtime = self.fixture(Path(directory))
            with self.assertRaisesRegex(ValueError, "release_tag_invalid"):
                runtime_bundle.stage(
                    source,
                    release,
                    output,
                    "aarch64-apple-darwin",
                    "macos-x86_64-1.2.3",
                    python_runtime,
                )
            with self.assertRaisesRegex(ValueError, "archive_tag_mismatch"):
                runtime_bundle.stage(
                    source,
                    release,
                    output,
                    "aarch64-apple-darwin",
                    f"{platform}-9.9.9",
                    python_runtime,
                )
            Path(f"{next(release.glob('*.tar.gz'))}.manifest.json.sig").unlink()
            with self.assertRaisesRegex(ValueError, "release_companion"):
                runtime_bundle.stage(
                    source,
                    release,
                    output,
                    "aarch64-apple-darwin",
                    f"{platform}-1.2.3",
                    python_runtime,
                )


if __name__ == "__main__":
    unittest.main()
