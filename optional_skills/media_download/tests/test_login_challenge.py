import base64
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "login_challenge.py"
PNG = b"\x89PNG\r\n\x1a\nfixture"


def load_module():
    spec = importlib.util.spec_from_file_location("media_download_login_challenge", ENTRYPOINT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class FakeClient:
    def __init__(self):
        self.calls = []
        self.next_id = 0

    def send(self, method, params=None):
        self.next_id += 1
        self.calls.append((self.next_id, method, params or {}))
        return self.next_id


class LoginChallengeTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.challenge = load_module()

    def test_publish_and_clear_use_only_private_non_secret_state(self):
        with tempfile.TemporaryDirectory() as directory:
            storage = Path(directory)
            profile = storage / "browser-profile" / "xiaohongshu"
            profile.mkdir(parents=True)
            client = FakeClient()

            def wait_result(command_id, timeout):
                self.assertGreater(timeout, 0)
                method = next(call[1] for call in client.calls if call[0] == command_id)
                if method == "Runtime.evaluate":
                    return {
                        "result": {
                            "result": {
                                "value": {"x": 10, "y": 20, "width": 220, "height": 220}
                            }
                        }
                    }
                return {"result": {"data": base64.b64encode(PNG).decode("ascii")}}

            self.assertTrue(
                self.challenge.publish_login_challenge(
                    client, wait_result, profile, "xiaohongshu"
                )
            )
            state_path = storage / "login_challenge" / "current.json"
            image_path = storage / "login_challenge" / "challenge.png"
            state = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertEqual(state["platform"], "xiaohongshu")
            self.assertEqual(state["status"], "awaiting_user")
            self.assertTrue(state["cropped"])
            self.assertNotIn("url", state)
            self.assertEqual(image_path.read_bytes(), PNG)
            self.assertEqual(state_path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(image_path.stat().st_mode & 0o777, 0o600)

            self.challenge.clear_login_challenge(profile, "xiaohongshu")
            self.assertFalse(state_path.exists())
            self.assertFalse(image_path.exists())

    def test_rejects_profiles_outside_the_skill_browser_profile_root(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "unexpected" / "xiaohongshu"
            profile.mkdir(parents=True)
            self.assertFalse(
                self.challenge.publish_login_challenge(
                    FakeClient(), lambda _id, _timeout: {}, profile, "xiaohongshu"
                )
            )

    def test_live_companion_minimizes_only_linux_login_browser(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory)
            state_dir = runtime / "agent-runtime"
            state_dir.mkdir()
            (state_dir / "login-challenge-companion.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "status": "active",
                        "device_class": "raspberry_pi",
                        "updated_at_epoch": 100.0,
                    }
                ),
                encoding="utf-8",
            )
            environment = {"XDG_RUNTIME_DIR": str(runtime)}
            args = self.challenge.companion_browser_args(environment, "linux", now=105.0)
            self.assertIn("--start-minimized", args)
            self.assertEqual(
                self.challenge.companion_browser_args(environment, "linux", now=120.0),
                [],
            )
            self.assertEqual(
                self.challenge.companion_browser_args(environment, "darwin", now=105.0),
                [],
            )

    def test_companion_capture_expands_small_virtual_viewport(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / "runtime"
            state_dir = runtime / "agent-runtime"
            state_dir.mkdir(parents=True)
            (state_dir / "login-challenge-companion.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "status": "active",
                        "device_class": "raspberry_pi",
                        "updated_at_epoch": self.challenge.time.time(),
                    }
                ),
                encoding="utf-8",
            )
            storage = root / "storage"
            profile = storage / "browser-profile" / "xiaohongshu"
            profile.mkdir(parents=True)
            client = FakeClient()

            def wait_result(command_id, timeout):
                self.assertGreater(timeout, 0)
                _, method, params = next(call for call in client.calls if call[0] == command_id)
                if method == "Runtime.evaluate" and "window.innerWidth" in params["expression"]:
                    return {
                        "result": {"result": {"value": {"width": 480, "height": 320}}}
                    }
                if method == "Runtime.evaluate":
                    return {
                        "result": {
                            "result": {
                                "value": {"x": 50, "y": 60, "width": 180, "height": 180}
                            }
                        }
                    }
                if method == "Page.captureScreenshot":
                    return {"result": {"data": base64.b64encode(PNG).decode("ascii")}}
                return {"result": {}}

            with mock.patch.dict(
                self.challenge.os.environ,
                {"XDG_RUNTIME_DIR": str(runtime)},
                clear=False,
            ):
                self.assertTrue(
                    self.challenge.publish_login_challenge(
                        client, wait_result, profile, "xiaohongshu"
                    )
                )
            resize_calls = [
                call for call in client.calls if call[1] == "Emulation.setDeviceMetricsOverride"
            ]
            self.assertEqual(len(resize_calls), 1)
            self.assertEqual(resize_calls[0][2]["width"], 960)
            self.assertEqual(resize_calls[0][2]["height"], 720)

    def test_companion_does_not_publish_an_unidentified_page_image(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / "runtime"
            state_dir = runtime / "agent-runtime"
            state_dir.mkdir(parents=True)
            (state_dir / "login-challenge-companion.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "status": "active",
                        "device_class": "raspberry_pi",
                        "updated_at_epoch": self.challenge.time.time(),
                    }
                ),
                encoding="utf-8",
            )
            profile = root / "storage" / "browser-profile" / "xiaohongshu"
            profile.mkdir(parents=True)
            client = FakeClient()

            def wait_result(command_id, _timeout):
                _, method, params = next(call for call in client.calls if call[0] == command_id)
                if method == "Runtime.evaluate" and "window.innerWidth" in params["expression"]:
                    return {"result": {"result": {"value": {"width": 960, "height": 720}}}}
                if method == "Runtime.evaluate":
                    return {"result": {"result": {"value": None}}}
                self.fail("an unidentified page must not be captured for the Pi companion")

            with mock.patch.dict(
                self.challenge.os.environ,
                {"XDG_RUNTIME_DIR": str(runtime)},
                clear=False,
            ):
                self.assertFalse(
                    self.challenge.publish_login_challenge(
                        client, wait_result, profile, "xiaohongshu"
                    )
                )

            state_path = root / "storage" / "login_challenge" / "current.json"
            self.assertFalse(state_path.exists())

    def test_companion_scan_timeout_and_expired_prompt_are_pi_only(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory)
            state_dir = runtime / "agent-runtime"
            state_dir.mkdir()
            (state_dir / "login-challenge-companion.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "status": "active",
                        "device_class": "raspberry_pi",
                        "updated_at_epoch": self.challenge.time.time(),
                    }
                ),
                encoding="utf-8",
            )
            environment = {"XDG_RUNTIME_DIR": str(runtime)}
            client = FakeClient()

            def expired_result(_command_id, _timeout):
                return {"result": {"result": {"value": True}}}

            self.assertEqual(
                self.challenge.effective_scan_timeout(600, environment),
                self.challenge.COMPANION_SCAN_TIMEOUT_SECONDS,
            )
            self.assertTrue(
                self.challenge.challenge_has_expired(client, expired_result, environment)
            )
            self.assertEqual(
                self.challenge.effective_scan_timeout(
                    600,
                    {"XDG_RUNTIME_DIR": str(runtime / "absent")},
                ),
                600,
            )
            self.assertFalse(
                self.challenge.challenge_has_expired(
                    client,
                    expired_result,
                    {"XDG_RUNTIME_DIR": str(runtime / "absent")},
                )
            )


if __name__ == "__main__":
    unittest.main()
