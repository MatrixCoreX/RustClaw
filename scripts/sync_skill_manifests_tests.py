import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

import sync_skill_manifests as sync


class RuntimeFilesTests(unittest.TestCase):
    def test_sync_preserves_package_owned_assets_without_a_host_skill_list(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.0.0"\n')
            source = root / "fixture"
            source.mkdir()
            skill = sync.CargoSkill("fixture", source, "fixture", "fixture")
            entry = {"name": "fixture", "kind": "runner"}
            rendered = sync.MARKER + '\n[build]\nlifecycle_scripts = false\n'
            files = [{"source": "helper.js", "destination": "runtime/assets/helper.js"}]
            path = source / "skill.toml"
            path.write_text(rendered + "runtime_files = " + sync.toml_literal(files) + "\n")
            with patch.object(sync, "ROOT", root), patch.object(sync, "render_manifest", return_value=rendered):
                sync.sync_manifests({"fixture": skill}, [entry], check=False)
                self.assertEqual(tomllib.loads(path.read_text())["build"]["runtime_files"], files)
                self.assertEqual(sync.sync_manifests({"fixture": skill}, [entry], check=True), 0)


if __name__ == "__main__":
    unittest.main()
