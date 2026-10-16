import json
from pathlib import Path
import tempfile
import unittest

from small_screen_login_challenge import (
    load_login_challenge,
    publish_login_companion_heartbeat,
)


class SmallScreenLoginChallengeTest(unittest.TestCase):
    def test_publishes_companion_heartbeat_in_volatile_runtime_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertTrue(
                publish_login_companion_heartbeat(
                    now=123.5,
                    environment={"XDG_RUNTIME_DIR": directory},
                )
            )
            state_path = Path(directory) / "agent-runtime" / "login-challenge-companion.json"
            state = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertEqual(state["schema_version"], 1)
            self.assertEqual(state["status"], "active")
            self.assertEqual(state["updated_at_epoch"], 123.5)
            self.assertEqual(state_path.stat().st_mode & 0o777, 0o600)

    def test_loads_fresh_private_challenge(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            challenge = root / "data" / "skills" / "media_download" / "login_challenge"
            challenge.mkdir(parents=True)
            (challenge / "challenge.png").write_bytes(b"\x89PNG\r\n\x1a\nfixture")
            (challenge / "current.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "status": "awaiting_user",
                        "platform": "xiaohongshu",
                        "image_file": "challenge.png",
                        "updated_at_epoch": 100,
                        "cropped": True,
                    }
                ),
                encoding="utf-8",
            )
            loaded = load_login_challenge(root / "data" / "skills", now=108)
            self.assertIsNotNone(loaded)
            self.assertEqual(loaded["platform"], "xiaohongshu")
            self.assertTrue(loaded["cropped"])
            self.assertGreater(loaded["image_mtime_ns"], 0)
            self.assertEqual(loaded["image_size_bytes"], 15)

    def test_rejects_stale_or_traversing_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            challenge = root / "data" / "skills" / "media_download" / "login_challenge"
            challenge.mkdir(parents=True)
            (challenge / "challenge.png").write_bytes(b"\x89PNG\r\n\x1a\nfixture")
            state_path = challenge / "current.json"
            state_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "status": "awaiting_user",
                        "platform": "xiaohongshu",
                        "image_file": "challenge.png",
                        "updated_at_epoch": 100,
                    }
                ),
                encoding="utf-8",
            )
            self.assertIsNone(load_login_challenge(root / "data" / "skills", now=200))
            state = json.loads(state_path.read_text(encoding="utf-8"))
            state["updated_at_epoch"] = 200
            state["image_file"] = "../challenge.png"
            state_path.write_text(json.dumps(state), encoding="utf-8")
            self.assertIsNone(load_login_challenge(root / "data" / "skills", now=200))


if __name__ == "__main__":
    unittest.main()
