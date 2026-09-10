from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from release_config_guard import credential_fields, inspect_config


ROOT = Path(__file__).resolve().parents[2]


class ReleaseConfigGuardTests(unittest.TestCase):
    def test_sensitive_values_are_rejected_recursively(self):
        for key in ("api_key", "app_secret", "bot_token", "private_key", "user_key", "password"):
            with self.subTest(key=key):
                self.assertEqual(credential_fields({"providers": [{key: "test-credential"}]}), [f"providers.0.{key}"])

    def test_safe_values_and_environment_references(self):
        for value in ("", "REPLACE_ME_TOKEN", "REDACTED_API_KEY", "${MODEL_API_KEY}"):
            self.assertEqual(credential_fields({"api_key": value}), [])
        self.assertEqual(credential_fields({"api_key_env": "MODEL_API_KEY"}), [])

    def test_normal_config_fields_are_not_secrets(self):
        self.assertEqual(credential_fields({
            "provider": "provider-fixture", "bot_enabled": True, "bot_count": 2,
            "session_id": "session-fixture", "release_artifact_id": "package-fixture",
            "forbid": ["command-fixture"], "max_tokens": 2048, "max_idle": 2,
        }), [])

    def test_parses_multiline_and_inline_credentials_without_exposing_values(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "config.toml"
            path.write_text('nested = [{ api_key = "secret-value" }]\nprivate_key = """\nsecret-multiline\n"""\n')
            findings = inspect_config(root, path)
            self.assertEqual(len(findings), 2)
            self.assertNotIn("secret-value", str(findings))
            self.assertNotIn("secret-multiline", str(findings))

    def test_configs_are_byte_preserved_for_both_product_identities(self):
        for fixture in (ROOT / "scripts/fixtures/product_identity").glob("*.toml"):
            with self.subTest(fixture=fixture.name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                path = root / "product_identity.toml"
                shutil.copyfile(fixture, path)
                before = path.read_bytes()
                self.assertEqual(inspect_config(root, path), [])
                self.assertEqual(path.read_bytes(), before)

    def test_channel_bindings_rejected_and_translations_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "channels").mkdir()
            channel = root / "channels/fixture.toml"
            channel.write_text('[channel]\nadmins = [123]\n')
            self.assertIn("private_channel_binding", str(inspect_config(root, channel)))
            (root / "i18n").mkdir()
            translation = root / "i18n/fixture.toml"
            translation.write_text('password = "Enter your password"\n')
            self.assertEqual(inspect_config(root, translation), [])

    def test_invalid_toml_and_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "invalid.toml"
            path.write_text('api_key = "unterminated-secret')
            self.assertEqual(inspect_config(root, path), ["invalid.toml: invalid_toml"])
            link = root / "linked.toml"
            link.symlink_to(path)
            self.assertEqual(inspect_config(root, link), ["linked.toml: symlink_not_allowed"])

    def test_host_temp_directory_alias_is_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            host = Path(directory)
            (host / "real/configs").mkdir(parents=True)
            (host / "alias").symlink_to(host / "real", target_is_directory=True)
            root = host / "alias/configs"
            path = root / "fixture.toml"
            path.write_text('provider = "provider-fixture"\n')
            self.assertEqual(inspect_config(root, path), [])

    def test_private_key_material_in_any_text_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "fixture.toml"
            path.write_text('# -----BEGIN PRIVATE KEY-----\nprovider = "fixture"\n')
            self.assertEqual(inspect_config(root, path), ["fixture.toml: private_key_material"])

    def test_cli_fails_closed_without_leaking_secret_or_changing_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "fixture.toml"
            original = 'api_key = "not-for-release-secret"\n'
            path.write_text(original)
            result = subprocess.run([sys.executable, str(ROOT / "scripts/security/release_config_guard.py"), str(root)], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("not-for-release-secret", result.stdout + result.stderr)
            self.assertEqual(path.read_text(), original)

    def test_package_script_uses_guard_before_archive(self):
        source = (ROOT / "package-release.sh").read_text()
        self.assertLess(source.index("scripts/security/release_config_guard.py"), source.index('tar -czf "$OUT"'))
        self.assertNotIn("REDACTED_BOT", source)


if __name__ == "__main__":
    unittest.main()
