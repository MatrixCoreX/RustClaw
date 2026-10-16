import json
from pathlib import Path
import tempfile
import unittest

from small_screen_login_challenge import (
    is_raspberry_pi_device,
    load_login_challenge,
    publish_login_companion_heartbeat,
)


class SmallScreenLoginChallengeTest(unittest.TestCase):
    def test_publishes_companion_heartbeat_in_volatile_runtime_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            model_path = Path(directory) / "model"
            model_path.write_bytes(b"Raspberry Pi 5 Model B Rev 1.1\x00")
            self.assertTrue(
                publish_login_companion_heartbeat(
                    now=123.5,
                    environment={"XDG_RUNTIME_DIR": directory},
                    model_path=model_path,
                )
            )
            state_path = Path(directory) / "agent-runtime" / "login-challenge-companion.json"
            state = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertEqual(state["schema_version"], 1)
            self.assertEqual(state["status"], "active")
            self.assertEqual(state["device_class"], "raspberry_pi")
            self.assertEqual(state["updated_at_epoch"], 123.5)
            self.assertEqual(state_path.stat().st_mode & 0o777, 0o600)

    def test_companion_heartbeat_is_restricted_to_raspberry_pi_hardware(self):
        with tempfile.TemporaryDirectory() as directory:
            model_path = Path(directory) / "model"
            model_path.write_text("Generic Linux Device", encoding="utf-8")
            self.assertFalse(is_raspberry_pi_device(model_path))
            self.assertFalse(
                publish_login_companion_heartbeat(
                    now=123.5,
                    environment={"XDG_RUNTIME_DIR": directory},
                    model_path=model_path,
                )
            )
            self.assertFalse(
                (Path(directory) / "agent-runtime" / "login-challenge-companion.json").exists()
            )

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
