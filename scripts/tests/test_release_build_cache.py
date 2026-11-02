"""Compiler caches belong to CI, stay target-aware, and save only success."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ("ubuntu-x86_64-release.yml", "pi-aarch64-release.yml",
             "macos-arm64-release-artifact.yml", "macos-polyglot-skill-ci.yml", "desktop-native.yml")


class BuildCacheTest(unittest.TestCase):
    def test_all_native_workflows_use_pinned_success_only_compiler_cache(self):
        for name in WORKFLOWS:
            with self.subTest(workflow=name):
                text = (ROOT / ".github/workflows" / name).read_text()
                self.assertIn("Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0", text)
                self.assertIn('cache-on-failure: "false"', text)
                self.assertIn('cache-bin: "false"', text)
                self.assertIn("save-if: ${{ github.event_name != 'pull_request' }}", text)

    def test_macos_release_caches_separate_runtime_and_desktop_workspaces(self):
        text = (ROOT / ".github/workflows/macos-arm64-release-artifact.yml").read_text()
        for value in (". -> target", "desktop -> target", "desktop/scripts/identity -> target", "MACOSX_DEPLOYMENT_TARGET SDKROOT"):
            self.assertIn(value, text)


if __name__ == "__main__":
    unittest.main()
