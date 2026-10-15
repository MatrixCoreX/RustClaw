"""macOS bundled skill dependency regressions."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "bundle_macos_skill_dependencies",
    ROOT / "scripts/bundle_macos_skill_dependencies.py",
)
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class BundleMacosSkillDependenciesTests(unittest.TestCase):
    def test_materialized_chromium_verifies_each_macho_directly(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "Chromium.app"
            executable = root / "Contents/MacOS/Chromium"
            framework = root / "Contents/Frameworks/Chromium Framework"
            executable.parent.mkdir(parents=True)
            framework.parent.mkdir(parents=True)
            executable.write_bytes(b"\xcf\xfa\xed\xfe" + bytes(28))
            framework.write_bytes(b"\xca\xfe\xba\xbe" + bytes(28))
            (root / "Contents/Info.plist").write_text("not a Mach-O")

            with mock.patch.object(bundle.subprocess, "run") as run:
                bundle.adhoc_sign_macho_files(root)

            self.assertEqual(run.call_count, 4)
            signed = run.call_args_list[::2]
            verified_calls = run.call_args_list[1::2]
            verified = {Path(call.args[0][-1]) for call in verified_calls}
            self.assertEqual(verified, {executable, framework})
            for call in signed:
                self.assertEqual(call.args[0][0:4], ["codesign", "--force", "--sign", "-"])
                self.assertIn("--preserve-metadata=identifier,entitlements,flags,runtime", call.args[0])
                self.assertTrue(call.kwargs["check"])
            for call in verified_calls:
                self.assertEqual(call.args[0][0:3], ["codesign", "--verify", "--strict"])
                self.assertTrue(call.kwargs["check"])

    def test_empty_macho_bundle_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "macos_macho_bundle_empty"):
                bundle.adhoc_sign_macho_files(Path(directory))


if __name__ == "__main__":
    unittest.main()
