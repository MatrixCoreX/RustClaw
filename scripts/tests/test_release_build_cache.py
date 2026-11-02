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

    def test_protocol_account_tests_install_the_skills_locked_node_dependencies_first(self):
        for name in ("macos-polyglot-skill-ci.yml", "ubuntu-x86_64-release.yml"):
            with self.subTest(workflow=name):
                text = (ROOT / ".github/workflows" / name).read_text()
                self.assertLess(text.index("npm ci --ignore-scripts --prefix optional_skills/media_discovery"),
                                text.index("node --test optional_skills/media_discovery/test/account_sessions.test.mjs"))

    def test_media_validation_has_one_automatic_entrypoint_and_covers_build_only_changes(self):
        child = (ROOT / ".github/workflows/media-session-portability-ci.yml").read_text()
        self.assertNotIn("\n  pull_request:", child)
        self.assertIn("workflow_call:", child)
        parent = (ROOT / ".github/workflows/macos-polyglot-skill-ci.yml").read_text()
        for name in ("scripts/build_macos_media_tools.sh", "scripts/bundle_macos_skill_dependencies.py",
                     "scripts/tests/test_release_macos_media_tools.py", "scripts/tests/test_release_build_cache.py",
                     ".github/workflows/media-session-portability-ci.yml"):
            self.assertEqual(parent.count(f'"{name}"'), 2, "both push and PR must retain the validation")


if __name__ == "__main__":
    unittest.main()
