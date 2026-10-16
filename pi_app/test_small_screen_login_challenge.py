import json
from pathlib import Path
import tempfile
import unittest

from small_screen_login_challenge import load_login_challenge


class SmallScreenLoginChallengeTest(unittest.TestCase):
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
