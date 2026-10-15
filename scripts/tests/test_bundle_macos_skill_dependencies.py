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
    def test_tool_launcher_is_native_and_uses_relocatable_paths(self):
        source = bundle.tool_launcher_source(
            "chromium/Chromium.app/Contents/MacOS/Chromium"
        )
        self.assertIn("_NSGetExecutablePath", source)
        self.assertIn('suffix = "/git/bin/git";', source)
        self.assertIn('setenv("GIT_EXEC_PATH"', source)
        self.assertIn('setenv("GIT_TEMPLATE_DIR"', source)
        self.assertIn('suffix = "/bin/tesseract.real";', source)
        self.assertIn('setenv("TESSDATA_PREFIX"', source)
        self.assertIn(
            'suffix = "/chromium/Chromium.app/Contents/MacOS/Chromium";', source
        )
        self.assertIn("execv(target, child_argv)", source)
        self.assertIn('fprintf(stderr, "bundled_tool_launcher_path_unavailable\\n");', source)
        self.assertIn("*name_separator = '\\0';", source)
        self.assertNotIn("\x00", source)
        self.assertNotIn("#!/bin/bash", source)

    def test_chromium_launcher_is_relocated_but_app_payload_is_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            launcher = root / "bin/chromium"
            chromium_binary = root / "chromium/Chromium.app/Contents/MacOS/Chromium"
            launcher.parent.mkdir(parents=True)
            chromium_binary.parent.mkdir(parents=True)
            launcher.write_bytes(b"launcher")
            chromium_binary.write_bytes(b"chromium")
            macho_paths = {launcher, chromium_binary}

            with (
                mock.patch.object(
                    bundle, "is_macho", side_effect=lambda path: path in macho_paths
                ),
                mock.patch.object(bundle, "linked_libraries", return_value=[]),
                mock.patch.object(bundle.subprocess, "run") as run,
            ):
                bundle.copy_external_libraries(root, {})

            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.args[0][-1], str(launcher))

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
            staged = {Path(call.args[0][-1]) for call in verified_calls}
            self.assertEqual(
                {path.name.split("-", 1)[1] for path in staged},
                {executable.name, framework.name},
            )
            self.assertTrue(all(not path.is_relative_to(root) for path in staged))
            for call in signed:
                self.assertEqual(call.args[0][0:4], ["codesign", "--force", "--sign", "-"])
                self.assertIn(
                    "--preserve-metadata=identifier,entitlements,flags,runtime",
                    call.args[0],
                )
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
