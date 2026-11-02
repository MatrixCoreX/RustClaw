"""Compiler caches belong to CI, stay target-aware, and save only success."""
from pathlib import Path, PurePosixPath
import re
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
        for value in (". -> target", "desktop -> target", "desktop/scripts/identity -> ../../target/identity",
                      "desktop/target/identity/debug", "MACOSX_DEPLOYMENT_TARGET SDKROOT"):
            self.assertIn(value, text)

    def test_only_cargo_profiles_are_cached_not_receipts_runtime_assets_or_installer_state(self):
        for name in (*WORKFLOWS, "rust-cache-profile-ci.yml"):
            with self.subTest(workflow=name):
                text = (ROOT / ".github/workflows" / name).read_text()
                self.assertIn('cache-targets: "false"', text)
                self.assertIn("prefix-key: v1-rust-cargo-profiles", text)
                roots = []
                for line in text.split("cache-directories: |\n", 1)[1].splitlines():
                    if not line.startswith("            "):
                        break
                    roots.append(re.sub(r"\$\{\{[^}]+\}\}", "aarch64-apple-darwin", line.strip()))
                self.assertTrue(roots)
                for root in roots:
                    self.assertIn(PurePosixPath(root).name, ("debug", "release"))
                    self.assertNotIn("*", root)
                    self.assertNotIn("..", PurePosixPath(root).parts)
                for artifact in ("target/prebuilt-skill-packages/a/skill/current.json",
                                 "target/skill-packages/a/skill/install-receipt.json",
                                 "target/prebuilt-skill-storage/a/session.json",
                                 "target/release-host-tools/a/bin/ffmpeg"):
                    self.assertFalse(any(PurePosixPath(artifact).is_relative_to(root) for root in roots))

    def test_successful_cache_is_restored_before_repeating_real_receipt_generation(self):
        child = (ROOT / ".github/workflows" / "media-session-portability-ci.yml").read_text()
        self.assertEqual(child.count("uses: ./.github/workflows/rust-cache-profile-ci.yml"), 2)
        self.assertIn("needs: cache-seed", child)
        self.assertIn("require_cache_hit: true", child)
        smoke = (ROOT / ".github/workflows" / "rust-cache-profile-ci.yml").read_text()
        self.assertIn("shared-key: compiler-profile-smoke", smoke)
        self.assertIn('[[ "$CACHE_HIT" == true ]]', smoke)
        self.assertIn("bash scripts/precompile_skill_store.sh host", smoke)
        self.assertIn("! -e target/prebuilt-skill-packages", smoke)

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
                     ".github/workflows/media-session-portability-ci.yml", ".github/workflows/rust-cache-profile-ci.yml"):
            self.assertEqual(parent.count(f'"{name}"'), 2, "both push and PR must retain the validation")


if __name__ == "__main__":
    unittest.main()
