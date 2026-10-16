import base64
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


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


if __name__ == "__main__":
    unittest.main()
