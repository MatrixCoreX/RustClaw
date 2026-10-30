"""Exercise the updater's actual file-application stage, without service hooks."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
RELEASE_FILES = (
    "deploy-ui-nginx.sh",
    "docs/release_installation.md",
    "docs/release_installation.zh-CN.md",
    "docs/developer_build.md",
)


class ReleaseManagedFilesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.installed = self.root / "installed"
        self.package = self.root / "package"
        self.work = self.root / "work"
        self.backup = self.root / "backup"
        for directory in (self.installed, self.package, self.work, self.backup):
            directory.mkdir()
        (self.root / "manifest.json").write_text(json.dumps({"release": {"commit": "a" * 40}}))

    def write(self, root, relative, content):
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content)
        return destination

    def apply(self):
        source = (ROOT / "deploy-github-release.sh").read_text()
        function_start = source.index("preserve_deployment_files() {")
        function_end = source.index('\nif [[ "$PACKAGE_MODE" -eq 1 ]]; then', function_start)
        stage_start = source.index('MANAGED_PATHS_FILE="$WORK_DIR/managed-paths.txt"')
        stage_end = source.index('bootstrap_bundled_release_skills "$ROOT_DIR" "$PACKAGE_DIR"', stage_start)
        variables = {
            "ROOT_DIR": self.installed, "PACKAGE_DIR": self.package,
            "WORK_DIR": self.work, "BACKUP_DIR": self.backup,
            "MANIFEST_RECORD": self.root / "manifest.json",
        }
        script = "set -euo pipefail\n" + "\n".join(
            f"{name}={shlex.quote(str(value))}" for name, value in variables.items()
        ) + "\n" + source[function_start:function_end] + source[stage_start:stage_end]
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-s"], input=script,
            capture_output=True, text=True, timeout=60,
        )

    def assert_applied(self):
        result = self.apply()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_packaged_documents_and_nginx_helper_replace_previous_versions(self):
        for relative in (*RELEASE_FILES, "README.md"):
            self.write(self.installed, relative, "previous\n")
            new = self.write(self.package, relative, "current\n")
            if relative.endswith(".sh"):
                new.chmod(0o755)
        self.assert_applied()
        managed = (self.work / "managed-paths.txt").read_text().splitlines()
        with tarfile.open(self.backup / "files.tar.gz") as backup:
            for relative in (*RELEASE_FILES, "README.md"):
                self.assertEqual((self.installed / relative).read_text(), "current\n", relative)
                self.assertIn(relative, managed)
                self.assertEqual(backup.extractfile(relative).read(), b"previous\n")
        self.assertTrue(os.access(self.installed / "deploy-ui-nginx.sh", os.X_OK))

    def test_missing_documents_are_installed_without_touching_custom_docs_or_configs(self):
        for relative in RELEASE_FILES:
            self.write(self.package, relative, "current\n")
        self.write(self.installed, "docs/local-notes.md", "user notes\n")
        self.write(self.installed, "configs/config.toml", "local = true\n")
        self.write(self.package, "configs/config.toml", "package = true\n")
        self.write(self.package, "configs/new-tool.toml", "new = true\n")
        self.assert_applied()
        for relative in RELEASE_FILES:
            self.assertEqual((self.installed / relative).read_text(), "current\n", relative)
        self.assertEqual((self.installed / "docs/local-notes.md").read_text(), "user notes\n")
        self.assertEqual((self.installed / "configs/config.toml").read_text(), "local = true\n")
        self.assertEqual((self.installed / "configs/new-tool.toml").read_text(), "new = true\n")
        self.assertEqual((self.backup / "new-config-paths.txt").read_text(), "configs/new-tool.toml\n")

    def test_absent_package_files_do_not_remove_existing_documents(self):
        for relative in RELEASE_FILES:
            self.write(self.installed, relative, "existing optional file\n")
        self.write(self.package, "README.md", "current\n")
        self.assert_applied()
        managed = (self.work / "managed-paths.txt").read_text().splitlines()
        for relative in RELEASE_FILES:
            self.assertEqual((self.installed / relative).read_text(), "existing optional file\n")
            self.assertNotIn(relative, managed)

    def test_missing_document_directory_is_created_without_touching_runtime_data(self):
        self.assertFalse((self.installed / "docs").exists())
        self.write(self.installed, "data/private-state", "preserved state\n")
        for relative in RELEASE_FILES:
            self.write(self.package, relative, "current\n")
        self.assert_applied()
        for relative in RELEASE_FILES:
            self.assertEqual((self.installed / relative).read_text(), "current\n")
        self.assertEqual((self.installed / "data/private-state").read_text(), "preserved state\n")
        with tarfile.open(self.backup / "files.tar.gz") as backup:
            self.assertEqual(backup.getnames(), [])

    def test_existing_scripts_ui_and_base_registry_still_update(self):
        for relative in ("scripts/fixture", "UI/dist/index.html", "configs/skills_registry.toml"):
            self.write(self.installed, relative, "previous\n")
            self.write(self.package, relative, "current\n")
        self.assert_applied()
        for relative in ("scripts/fixture", "UI/dist/index.html", "configs/skills_registry.toml"):
            self.assertEqual((self.installed / relative).read_text(), "current\n")

    def test_declared_root_files_and_guides_do_not_drift_from_the_updater(self):
        declared = set()
        for line in (ROOT / "package-release.sh").read_text().splitlines():
            if not line.startswith("copy_if_exists "):
                continue
            command = shlex.split(line)
            self.assertEqual(len(command), 2)
            relative = Path(command[1])
            if (ROOT / relative).is_file() and (len(relative.parts) == 1 or relative.parts[0] == "docs"):
                declared.add(relative.as_posix())
        self.assertTrue(set(RELEASE_FILES).issubset(declared))
        for relative in declared:
            self.write(self.installed, relative, "previous\n")
            self.write(self.package, relative, "current\n")
        self.assert_applied()
        for relative in declared:
            self.assertEqual((self.installed / relative).read_text(), "current\n", relative)

    def test_document_parent_cannot_escape_installation_before_any_managed_write(self):
        outside = self.root / "outside"
        outside.mkdir()
        self.write(outside, "release_installation.md", "outside unchanged\n")
        (self.installed / "docs").symlink_to(outside, target_is_directory=True)
        self.write(self.package, "docs/release_installation.md", "current\n")
        self.write(self.installed, "scripts/fixture", "previous\n")
        self.write(self.package, "scripts/fixture", "current\n")
        result = self.apply()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("release_preservation_parent_invalid", result.stdout + result.stderr)
        self.assertEqual((outside / "release_installation.md").read_text(), "outside unchanged\n")
        self.assertEqual((self.installed / "scripts/fixture").read_text(), "previous\n")
        self.assertFalse((self.backup / "files.tar.gz").exists())

    def test_alias_to_document_parent_inside_installation_is_allowed(self):
        inside = self.installed / "manuals"
        inside.mkdir()
        (self.installed / "docs").symlink_to(inside, target_is_directory=True)
        self.write(inside, "release_installation.md", "previous\n")
        self.write(self.package, "docs/release_installation.md", "current\n")
        self.assert_applied()
        self.assertTrue((self.installed / "docs").is_symlink())
        self.assertEqual((inside / "release_installation.md").read_text(), "current\n")

    def test_existing_relocated_runtime_data_layout_is_unchanged(self):
        relocated = self.root / "host-owned-data"
        relocated.mkdir()
        (self.installed / "data").symlink_to(relocated, target_is_directory=True)
        self.write(relocated, "skill-packages/fixture/program", "previous program\n")
        self.write(self.package, "data/skill-packages/fixture/program", "current program\n")
        self.write(relocated, "host-private-state", "preserved state\n")
        self.assert_applied()
        self.assertTrue((self.installed / "data").is_symlink())
        self.assertEqual((relocated / "skill-packages/fixture/program").read_text(), "current program\n")
        self.assertEqual((relocated / "host-private-state").read_text(), "preserved state\n")


if __name__ == "__main__":
    unittest.main()
