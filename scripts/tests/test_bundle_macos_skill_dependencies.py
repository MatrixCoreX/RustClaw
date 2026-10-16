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
    def test_tool_check_environment_preserves_host_and_adds_bundle_values(self):
        with mock.patch.object(bundle.subprocess, "run") as run:
            run.return_value.stdout = "git version fixture\n"
            run.return_value.stderr = ""
            output = bundle.run_with_environment(
                ("/tmp/runtime with spaces/git/bin/git", "--version"),
                {"GIT_EXEC_PATH": "/tmp/runtime with spaces/git/libexec/git-core"},
            )

        self.assertEqual(output, "git version fixture\n")
        self.assertEqual(
            run.call_args.kwargs["env"]["GIT_EXEC_PATH"],
            "/tmp/runtime with spaces/git/libexec/git-core",
        )
        self.assertIn("PATH", run.call_args.kwargs["env"])

    def test_direct_tool_is_relocated_but_chromium_app_payload_is_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            direct_tool = root / "bin/tesseract-real"
            chromium_binary = root / "chromium/Chromium.app/Contents/MacOS/Chromium"
            direct_tool.parent.mkdir(parents=True)
            chromium_binary.parent.mkdir(parents=True)
            direct_tool.write_bytes(b"tesseract")
            chromium_binary.write_bytes(b"chromium")
            macho_paths = {direct_tool, chromium_binary}

            with (
                mock.patch.object(
                    bundle, "is_macho", side_effect=lambda path: path in macho_paths
                ),
                mock.patch.object(bundle, "linked_libraries", return_value=[]),
                mock.patch.object(bundle.subprocess, "run") as run,
            ):
                bundle.copy_external_libraries(root, {})

            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.args[0][-1], str(direct_tool))

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

    def test_chromium_command_shim_resolves_bundle_with_spaces(self):
        with tempfile.TemporaryDirectory(prefix="macos host tools ") as directory:
            root = Path(directory)
            executable = root / "chromium/Chromium.app/Contents/MacOS/Chromium"
            executable.parent.mkdir(parents=True)
            executable.write_text("#!/bin/sh\nprintf '%s\\n' \"$*\"\n")
            executable.chmod(0o755)
            (root / "chromium-executable-relative").write_text(
                "chromium/Chromium.app/Contents/MacOS/Chromium\n"
            )

            shim = bundle.write_chromium_command_shim(root)
            completed = bundle.subprocess.run(
                [str(shim), "--version", "fixture value"],
                check=True,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.stdout, "--version fixture value\n")
            self.assertEqual(shim.stat().st_mode & 0o777, 0o755)

    def test_tesseract_command_shim_uses_bundle_data_with_spaces(self):
        with tempfile.TemporaryDirectory(prefix="macos host tools ") as directory:
            root = Path(directory)
            executable = root / "bin/tesseract-real"
            tessdata = root / "share/tessdata"
            executable.parent.mkdir(parents=True)
            tessdata.mkdir(parents=True)
            executable.write_text(
                "#!/bin/sh\nprintf '%s|%s\\n' \"$TESSDATA_PREFIX\" \"$*\"\n"
            )
            executable.chmod(0o755)

            shim = bundle.write_tesseract_command_shim(root)
            environment = bundle.os.environ.copy()
            environment["TESSDATA_PREFIX"] = "/wrong/tessdata"
            completed = bundle.subprocess.run(
                [str(shim), "--list-langs"],
                check=True,
                capture_output=True,
                text=True,
                env=environment,
            )

            self.assertEqual(
                completed.stdout,
                f"{root.resolve() / 'share/tessdata'}|--list-langs\n",
            )
            self.assertEqual(shim.stat().st_mode & 0o777, 0o755)


if __name__ == "__main__":
    unittest.main()
