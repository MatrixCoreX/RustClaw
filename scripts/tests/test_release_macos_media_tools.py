"""Offline contracts for portable media builds; native execution is CI-only."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("media_bundle", ROOT / "scripts/bundle_macos_skill_dependencies.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class MediaPortabilityTest(unittest.TestCase):
    def test_modern_minos_and_legacy_version_ignore_unrelated_dylinker_versions(self):
        output = """Load command 0
          cmd LC_BUILD_VERSION
          minos 13.0
          sdk 15.4
        Load command 1
          cmd LC_SOURCE_VERSION
          version 1115.7.3
        Load command 2
          cmd LC_VERSION_MIN_MACOSX
          version 12.3
          sdk 13.1
        """
        self.assertEqual(module.media_deployment_versions(output), [(13, 0, 0), (12, 3, 0)])

    def test_newer_or_absent_deployment_metadata_is_rejected(self):
        for output in ("Load command 0\n cmd LC_BUILD_VERSION\n minos 14.0\n", ""):
            with self.subTest(output=output), patch.object(module, "run", return_value=output):
                with self.assertRaisesRegex(ValueError, "deployment_target_invalid"):
                    module.verify_media_tool_portability(Path("ffmpeg"), "13.0")

    def test_only_system_libraries_are_allowed_after_static_source_build(self):
        def output(*args):
            if args[1] == "-l":
                return "Load command 0\n cmd LC_BUILD_VERSION\n minos 13.0\n"
            return "ffmpeg:\n /usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n"
        with patch.object(module, "run", side_effect=output):
            module.verify_media_tool_portability(Path("ffmpeg"), "13.0")
        with patch.object(module, "run", side_effect=output), patch.object(module, "linked_libraries", return_value=["@rpath/libavdevice.dylib"]):
            with self.assertRaisesRegex(ValueError, "external_dependency"):
                module.verify_media_tool_portability(Path("ffmpeg"), "13.0")

    def test_release_uses_pinned_ci_source_build_instead_of_ffmpeg_bottles(self):
        workflow = (ROOT / ".github/workflows/macos-arm64-release-artifact.yml").read_text()
        self.assertIn("bash scripts/build_macos_media_tools.sh", workflow)
        self.assertNotIn('$(brew --prefix ffmpeg)', workflow)
        self.assertIn('--minimum-macos-version "$MACOSX_DEPLOYMENT_TARGET"', workflow)
        builder = (ROOT / "scripts/build_macos_media_tools.sh").read_text()
        self.assertIn('"${GITHUB_ACTIONS:-}" == true', builder)
        self.assertIn("verify_media_tool_portability", builder)
        self.assertIn("--enable-libx264", builder)
        self.assertIn("--enable-libdav1d", builder)
        self.assertIn("--enable-libmp3lame", builder)
        self.assertIn("--enable-securetransport", builder)
        self.assertIn("--disable-autodetect", builder)


if __name__ == "__main__":
    unittest.main()
