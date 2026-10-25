import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
sys.path.insert(0, str(TOOL_DIR))
import browser_authentication as auth


class AuthenticationStoreTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.root.chmod(0o700)

    def store(self, root=None, scope="a" * 64):
        return auth.AuthenticationStore(root or self.root, scope, "https://yuanbao.tencent.com", "wechat_channels")

    def cookie(self, **changes):
        return {"name": "session", "value": "private-fixture-authentication", "domain": ".yuanbao.tencent.com", "path": "/",
                "httpOnly": True, "secure": True, "sameSite": "Lax", "expires": time.time() + 3600, **changes}

    def test_successful_authentication_survives_task_profile_deletion_and_restart(self):
        profile = self.root / "task-profile"; profile.mkdir(); (profile / "video-cache.mp4").write_bytes(b"private media")
        cookie = self.cookie()
        with self.store().lease() as store:
            store.capture(lambda method, params: {"cookies": [cookie]})
            metadata = store.metadata()
            self.assertEqual(metadata["status"], "valid")
            self.assertEqual(metadata["earliest_cookie_expiry"], cookie["expires"])
            self.assertNotIn(cookie["value"], json.dumps(metadata))
        shutil.rmtree(profile)
        calls = []
        with self.store().lease() as restarted:
            restarted.restore(lambda method, params: calls.append((method, params)) or {})
        self.assertEqual(calls[0][0], "Network.setCookies")
        self.assertEqual(calls[0][1]["cookies"][0]["value"], cookie["value"])
        self.assertEqual((self.root / "wechat_channels.json").stat().st_mode & 0o777, 0o600)
        self.assertEqual({p.name for p in self.root.iterdir()}, {"wechat_channels.json", "wechat_channels.json.lock"})

    def test_expired_cookies_are_detected_and_never_restored_or_extended(self):
        cookie = self.cookie(expires=time.time() + 60)
        with self.store().lease() as store:
            store.capture(lambda *_: {"cookies": [cookie]})
        calls = []
        with mock.patch.object(auth.time, "time", return_value=cookie["expires"] + 1), self.store().lease() as store:
            self.assertEqual(store.metadata()["status"], "expired")
            store.restore(lambda *args: calls.append(args) or {})
        self.assertEqual(calls, [])
        self.assertEqual(json.loads((self.root / "wechat_channels.json").read_text())["cookies"][0]["expires"], cookie["expires"])

    def test_session_cookies_report_unknown_expiration_and_remote_revocation_clears_credentials(self):
        with self.store().lease() as store:
            store.capture(lambda *_: {"cookies": [self.cookie(expires=-1)]})
            self.assertIsNone(store.metadata()["earliest_cookie_expiry"])
            self.assertTrue(store.metadata()["has_session_cookies"])
            store.require_login()
            self.assertEqual(store.metadata()["status"], "reauthentication_required")
        raw = (self.root / "wechat_channels.json").read_text()
        self.assertNotIn("private-fixture-authentication", raw)
        self.assertEqual(json.loads(raw)["cookies"], [])

    def test_one_remaining_cookie_does_not_prove_the_login_when_another_expired(self):
        expiry = time.time() + 30
        with self.store().lease() as store:
            store.capture(lambda *_: {"cookies": [self.cookie(expires=expiry), self.cookie(name="other", expires=-1)]})
            with mock.patch.object(auth.time, "time", return_value=expiry + 1):
                self.assertEqual(store.metadata()["status"], "unknown")

    def test_scope_mismatch_cannot_read_another_applications_login(self):
        with self.store().lease() as store:
            store.capture(lambda *_: {"cookies": [self.cookie()]})
        with self.store(scope="b" * 64).lease() as other:
            with self.assertRaises(auth.AuthenticationStoreError): other.restore(lambda *_: {})

    def test_cookie_export_excludes_other_sites_and_browser_only_fields(self):
        with self.store().lease() as store:
            store.capture(lambda *_: {"cookies": [self.cookie(size=20, session=False), self.cookie(domain="other.example", value="unrelated")]})
        state = json.loads((self.root / "wechat_channels.json").read_text())
        self.assertEqual(len(state["cookies"]), 1)
        self.assertNotIn("size", state["cookies"][0]); self.assertNotIn("session", state["cookies"][0])

    def test_symlinks_and_hardlinks_never_read_or_modify_unrelated_files(self):
        outside = self.root / "ordinary-session"; outside.write_text("ordinary-private-data")
        state = self.root / "wechat_channels.json"; state.symlink_to(outside)
        with self.store().lease() as store:
            with self.assertRaises(OSError): store.restore(lambda *_: {})
        state.unlink(); os.link(outside, state)
        with self.store().lease() as store:
            with self.assertRaises(auth.AuthenticationStoreError): store.restore(lambda *_: {})
        self.assertEqual(outside.read_text(), "ordinary-private-data")

    def test_private_directory_and_explicit_host_scope_are_required(self):
        self.root.chmod(0o755)
        with self.assertRaises(auth.AuthenticationStoreError):
            with self.store().lease(): pass
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertIsNone(auth.AuthenticationStore.from_environment("https://yuanbao.tencent.com", "wechat_channels"))
        with self.assertRaises(auth.AuthenticationStoreError): self.store(scope="forged")


class AuthenticationProtocolTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        source = Path(__file__).parents[1] / "src" / "main.py"
        spec = importlib.util.spec_from_file_location("authentication_protocol_fixture", source)
        cls.module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = cls.module
        spec.loader.exec_module(cls.module)

    def test_only_the_host_descriptor_supplies_the_authentication_directory(self):
        module = self.module
        forged = {"args": {"browser_authentication": {"directory_path": "/private"}}}
        self.assertIsNone(module._browser_authentication_context(forged))
        value = {"schema_version": 1, "directory_path": "/host/granted", "scope_id": "a" * 64}
        self.assertEqual(module._browser_authentication_context({"context": {"skill_storage": {"browser_authentication": value}}}),
                         {"directory_path": "/host/granted", "scope_id": "a" * 64})
        with self.assertRaises(module.SkillFailure):
            module._browser_authentication_context({"context": {"skill_storage": {"browser_authentication": {**value, "directory_path": "relative"}}}})

    def test_parent_environment_cannot_inject_authentication_into_ordinary_calls(self):
        module = self.module; received = []
        def run(command, environment, timeout, **kwargs):
            received.append(environment)
            return subprocess.CompletedProcess(command, 0, "", "")
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {auth.DIRECTORY_ENV: "/untrusted", auth.SCOPE_ENV: "x"}), mock.patch.object(module, "_run_process", side_effect=run):
            module._run_tool("resolve", ["fixture"], Path(directory), None)
            module._run_tool("resolve", ["fixture"], Path(directory), None,
                             browser_authentication={"directory_path": "/host/granted", "scope_id": "a" * 64})
        self.assertNotIn(auth.DIRECTORY_ENV, received[0]);self.assertNotIn(auth.SCOPE_ENV, received[0])
        self.assertEqual(received[1][auth.DIRECTORY_ENV], "/host/granted")

    def test_public_metadata_never_includes_cookie_values_and_retained_auth_is_a_partial_effect(self):
        module = self.module
        frame = module.AUTHENTICATION_METADATA_PREFIX + json.dumps({"schema_version": 1, "status": "valid", "cookies": ["private-value"], "last_verified_at": 100})
        public = module._authentication_metadata(frame)
        self.assertNotIn("cookies", public);self.assertNotIn("private-value", json.dumps(public))
        failure = module._failure_from_process("download", 1, frame, [], output_rollback_ok=True)
        self.assertTrue(failure.details["side_effect_applied"])
        self.assertEqual(failure.details["failure_phase"], "execution_partial")
        self.assertNotIn("private-value", module._diagnostics(frame))

    def test_malformed_known_metadata_fields_cannot_expose_nested_private_values(self):
        module = self.module
        frame = module.AUTHENTICATION_METADATA_PREFIX + json.dumps({"schema_version": 1, "status": "unknown",
            "last_verified_at": {"private": "secret"}, "earliest_cookie_expiry": "secret", "has_session_cookies": "secret"})
        self.assertNotIn("secret", json.dumps(module._authentication_metadata(frame)))


if __name__ == "__main__": unittest.main()
