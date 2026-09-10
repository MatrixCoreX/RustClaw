import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from stage_release_skill_sources import copy_tracked_source, stage_sources


class ReleaseSkillSourcesTests(unittest.TestCase):
    def test_tracked_sources_and_lockfiles_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            source = root / "optional_skills/fixture"
            source.mkdir(parents=True)
            for name in ("skill.toml", "package-lock.json", "main.mjs"):
                (source / name).write_text("fixture\n")
            subprocess.run(["git", "-C", str(root), "add", "optional_skills"], check=True)
            (source / "untracked-secret.txt").write_text("not-for-release\n")
            (source / "node_modules").mkdir()
            (source / "node_modules/cache").write_text("cache\n")
            stage = root / "stage"
            self.assertEqual(copy_tracked_source(root, source, stage, "package-lock.json"), 3)
            staged = stage / "optional_skills/fixture"
            self.assertTrue((staged / "main.mjs").is_file())
            self.assertFalse((staged / "untracked-secret.txt").exists())
            self.assertFalse((staged / "node_modules").exists())
            with self.assertRaisesRegex(ValueError, "contract_missing"):
                copy_tracked_source(root, source, stage, "missing.lock")

    def test_platform_packages_contain_source_and_manifest_without_core_source(self):
        for target in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin"):
            with self.subTest(target=target), tempfile.TemporaryDirectory() as directory:
                stage = Path(directory)
                count = stage_sources(ROOT, stage, target)
                self.assertGreater(count, 0)
                self.assertEqual(len(list(stage.glob("optional_skills/*/skill.toml"))), count)
                self.assertTrue(list(stage.glob("optional_skills/*/src/*")))
                self.assertFalse((stage / "crates/clawd").exists())
                self.assertFalse((stage / "data").exists())
                self.assertFalse(list(stage.rglob("node_modules")))

    def test_package_script_stages_sources_before_archive(self):
        source = (ROOT / "package-release.sh").read_text()
        self.assertLess(source.index("stage_release_skill_sources.py"), source.index('tar -czf "$OUT"'))


if __name__ == "__main__":
    unittest.main()
