"""Exercise the installed shell entry against actual Git index contents."""
from pathlib import Path
import importlib.util
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
ENTRY = ROOT / "scripts/check-secrets.sh"


class StagedSecretTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="staged-secret-test-")
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name)
        self.environment = dict(os.environ, GIT_CONFIG_NOSYSTEM="1")
        self.environment.pop("GIT_INDEX_FILE", None)
        self.environment.pop("GIT_DIR", None)
        self.environment.pop("GIT_WORK_TREE", None)
        self.git("init", "--quiet")

    def git(self, *args, input=None):
        return subprocess.run(["git", *args], cwd=self.repo, env=self.environment,
                              check=True, capture_output=True, text=True, input=input)

    def config(self, name="settings.toml", value="REPLACE_ME"):
        path = self.repo / "configs" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f'api_key = "{value}"\n', encoding="utf-8")
        return path

    def check(self, *args):
        return subprocess.run(["bash", str(ENTRY), *args], cwd=self.repo,
                              env=self.environment, capture_output=True, text=True, timeout=5)

    def test_staged_secret_is_rejected_after_worktree_is_sanitized(self):
        secret = "sk-" + "synthetic_stage_guard_" * 3
        path = self.config(value=secret)
        self.git("add", "--", str(path.relative_to(self.repo)))
        self.config(value="REPLACE_ME")
        result = self.check("--staged")
        self.assertEqual(result.returncode, 1, result.stderr)

    def test_safe_index_is_not_rejected_by_unstaged_secret(self):
        path = self.config()
        self.git("add", "--", str(path.relative_to(self.repo)))
        self.config(value="sk-" + "synthetic_unstaged_only_" * 3)
        result = self.check("--staged")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_finding_does_not_echo_secret(self):
        secret = "sk-" + "synthetic_output_guard_" * 3
        path = self.config(value=secret)
        self.git("add", "--", str(path.relative_to(self.repo)))
        result = self.check("--staged")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertNotIn(secret, result.stdout + result.stderr)
        self.assertIn("settings.toml", result.stdout + result.stderr)

    def test_secret_in_staged_newline_filename_is_rejected(self):
        path = self.config("name\nwith space.toml", "sk-" + "synthetic_filename_" * 3)
        self.git("add", "--", str(path.relative_to(self.repo)))
        result = self.check("--staged")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertNotIn("name\nwith space", result.stdout)
        self.assertIn("name\\nwith space", result.stdout)

    def test_index_is_checked_even_if_worktree_file_is_deleted(self):
        path = self.config(value="sk-" + "synthetic_deleted_worktree_" * 3)
        self.git("add", "--", str(path.relative_to(self.repo)))
        path.unlink()
        self.assertEqual(self.check("--staged").returncode, 1)

    def test_empty_index_does_not_fall_back_to_unstaged_configs(self):
        self.config(value="sk-" + "synthetic_empty_index_" * 3)
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("files=0", result.stdout)

    def test_supported_placeholders_and_empty_values_remain_valid(self):
        path = self.config()
        placeholders = ["", "REPLACE_ME", "REPLACE_ME_PROVIDER", "your_provider_key_here",
                        "${PROVIDER_KEY}", "$PROVIDER_KEY", "********"]
        path.write_text("".join(f'api_key = "{value}"\n' for value in placeholders), encoding="utf-8")
        self.git("add", "--", str(path.relative_to(self.repo)))
        self.assertEqual(self.check().returncode, 0)

    def test_placeholder_in_comment_does_not_hide_a_real_value(self):
        path = self.config(value="synthetic_sensitive_literal_" * 3)
        with path.open("a", encoding="utf-8") as output:
            output.write("# REPLACE_ME\n")
        text = path.read_text(encoding="utf-8").replace("\n#", " #", 1)
        path.write_text(text, encoding="utf-8")
        self.git("add", "--", str(path.relative_to(self.repo)))
        self.assertEqual(self.check().returncode, 1)

    def test_yaml_quotes_and_environment_assignments_are_scanned(self):
        for name, text in (
            ("entry.yaml", "access_token: '" + "synthetic_yaml_value_" * 3 + "'\n"),
            ("entry.env", "password=" + "synthetic_env_value_" * 3 + "\n"),
            ("entry.yml", '"app_secret": "' + "synthetic_json_style_" * 3 + '"\n'),
        ):
            with self.subTest(name=name):
                path = self.config(name)
                path.write_text(text, encoding="utf-8")
        self.git("add", "configs")
        result = self.check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("findings=3", result.stdout)

    def test_private_key_marker_has_only_redacted_diagnostic(self):
        path = self.config()
        path.write_text('private_key = """\n-----BEGIN PRIVATE KEY-----\n"""\n', encoding="utf-8")
        self.git("add", "configs")
        result = self.check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("private_key_material", result.stdout)
        self.assertNotIn("-----BEGIN", result.stdout + result.stderr)

    def test_staged_configuration_symlink_is_refused_without_following_it(self):
        outside = self.repo / "not-scanned.txt"
        outside.write_text("outside sentinel", encoding="utf-8")
        path = self.repo / "configs/settings.toml"
        path.parent.mkdir()
        path.symlink_to(outside)
        self.git("add", "configs")
        result = self.check()
        self.assertEqual(result.returncode, 2)
        self.assertIn("configuration_not_regular_blob", result.stderr)
        self.assertNotIn("outside sentinel", result.stdout + result.stderr)
        self.assertEqual(outside.read_text(), "outside sentinel")

    def test_non_utf8_configuration_is_not_silently_skipped(self):
        path = self.config()
        path.write_bytes(b"\xff\x80")
        self.git("add", "configs")
        result = self.check()
        self.assertEqual(result.returncode, 2)
        self.assertIn("configuration_not_utf8", result.stderr)

    def test_configuration_larger_than_bound_is_not_silently_skipped(self):
        path = self.config()
        with path.open("wb") as output:
            output.truncate(16 * 1024 * 1024 + 1)
        self.git("add", "configs")
        result = self.check()
        self.assertEqual(result.returncode, 2)
        self.assertIn("configuration_too_large", result.stderr)

    def test_missing_git_object_fails_closed(self):
        path = self.config()
        self.git("add", "configs")
        oid = self.git("rev-parse", ":configs/settings.toml").stdout.strip()
        (self.repo / ".git/objects" / oid[:2] / oid[2:]).unlink()
        self.assertEqual(self.check().returncode, 2)

    def test_explicit_path_checks_worktree_only_when_requested(self):
        path = self.config(value="sk-" + "synthetic_explicit_value_" * 3)
        result = self.check(str(path))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.check().returncode, 0)

    def test_staged_and_explicit_modes_cannot_be_mixed(self):
        path = self.config()
        self.assertEqual(self.check("--staged", str(path)).returncode, 2)
        self.assertEqual(self.check("--sta").returncode, 2)

    def test_install_does_not_replace_an_existing_hook(self):
        hook = self.repo / ".git/hooks/pre-commit"
        hook.write_text("original hook sentinel", encoding="utf-8")
        result = self.check("--install")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(hook.read_text(), "original hook sentinel")

    def test_installed_hook_runs_the_same_index_guard(self):
        self.assertEqual(self.check("--install").returncode, 0)
        hook = self.repo / ".git/hooks/pre-commit"
        path = self.config(value="sk-" + "synthetic_installed_hook_" * 3)
        self.git("add", "configs")
        result = subprocess.run([str(hook)], cwd=self.repo, env=self.environment,
                                capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertIn("SECRET_GUARD", result.stdout)
        self.assertEqual(path.read_text().count("synthetic_installed_hook_"), 3)

    def test_unmerged_index_is_an_explicit_failure(self):
        path = self.config()
        oid = self.git("hash-object", "-w", str(path)).stdout.strip()
        self.git("update-index", "--index-info",
                 input=f"100644 {oid} 1\tconflict.toml\n100644 {oid} 2\tconflict.toml\n")
        result = self.check()
        self.assertEqual(result.returncode, 2)
        self.assertIn("unmerged_index", result.stderr)

    def test_index_scan_from_repository_subdirectory_uses_full_paths(self):
        path = self.config(value="sk-" + "synthetic_subdirectory_" * 3)
        self.git("add", "configs")
        result = subprocess.run(["bash", str(ENTRY), "--staged"], cwd=path.parent,
                                env=self.environment, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertIn("configs/settings.toml", result.stdout)

    def test_selected_object_remains_fixed_after_index_changes(self):
        secret = "sk-" + "synthetic_fixed_object_" * 3
        self.config(value=secret)
        self.git("add", "configs")
        spec = importlib.util.spec_from_file_location("staged_guard", ENTRY.with_name("check_staged_secrets.py"))
        guard = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(guard)
        original = guard.git

        def replace_index_after_selection(root, *args):
            result = original(root, *args)
            if args[0] == "diff":
                self.config(value="REPLACE_ME")
                self.git("add", "configs")
            return result

        with patch.object(guard, "git", side_effect=replace_index_after_selection):
            selected = list(guard.staged_blobs(self.repo))
        self.assertEqual(len(selected), 1)
        self.assertIn(secret.encode(), selected[0][1])
        self.assertEqual(self.check().returncode, 0)

    def commit_fixture(self):
        self.git("add", "configs")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "--quiet", "-m", "configuration fixture")

    def test_regular_to_symlink_type_change_is_also_refused(self):
        path = self.config()
        self.commit_fixture()
        path.unlink()
        path.symlink_to("../outside.txt")
        self.git("add", "configs")
        result = self.check()
        self.assertEqual(result.returncode, 2)
        self.assertIn("configuration_not_regular_blob", result.stderr)

    def test_removal_does_not_scan_deleted_object_from_head(self):
        self.config(value="sk-" + "synthetic_deleted_index_" * 3)
        self.commit_fixture()
        self.git("rm", "--quiet", "configs/settings.toml")
        result = self.check()
        self.assertEqual(result.returncode, 0)
        self.assertIn("files=0", result.stdout)

    def test_renamed_configuration_checks_new_blob(self):
        path = self.config()
        self.commit_fixture()
        self.git("mv", "configs/settings.toml", "configs/renamed.yaml")
        path = path.with_name("renamed.yaml")
        path.write_text("token: " + "synthetic_rename_literal_" * 3 + "\n", encoding="utf-8")
        self.git("add", "configs")
        result = self.check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("renamed.yaml", result.stdout)

    @unittest.skipUnless(hasattr(os, "mkfifo"), "requires POSIX FIFO")
    def test_explicit_fifo_is_refused_without_blocking(self):
        path = self.repo / "pipe.toml"
        os.mkfifo(path)
        result = self.check(str(path))
        self.assertEqual(result.returncode, 2)
        self.assertIn("configuration_not_regular_file", result.stderr)


if __name__ == "__main__":
    unittest.main()
