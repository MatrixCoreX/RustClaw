import json
from pathlib import Path
import tempfile
import unittest

from small_screen_login_challenge import (
    is_raspberry_pi_device,
    load_login_challenge,
    publish_login_companion_heartbeat,
    request_login_challenge_browser,
)
from small_screen_strings import STRINGS


class SmallScreenLoginChallengeTest(unittest.TestCase):
    def test_kuaishou_login_challenge_has_localized_platform_name(self):
        self.assertEqual(STRINGS["CN"]["login_platform_kuaishou"], "快手")
        self.assertEqual(STRINGS["EN"]["login_platform_kuaishou"], "Kuaishou")

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

    def test_newest_skill_private_challenge_supports_browser_handoff(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "data" / "skills"
            for skill, updated in (("media_download", 100), ("media_discovery", 105)):
                challenge = root / skill / "login_challenge"
                challenge.mkdir(parents=True)
                (challenge / "challenge.png").write_bytes(
                    b"\x89PNG\r\n\x1a\nfixture"
                )
                state = {
                    "schema_version": 1,
                    "status": "awaiting_user",
                    "platform": "douyin",
                    "image_file": "challenge.png",
                    "updated_at_epoch": updated,
                }
                if skill == "media_discovery":
                    state.update(
                        {
                            "handoff_id": "12345678-1234-1234-1234-123456789abc",
                            "interaction": "browser_window",
                            "reason_code": "challenge_required",
                        }
                    )
                (challenge / "current.json").write_text(
                    json.dumps(state), encoding="utf-8"
                )
            loaded = load_login_challenge(root, now=108)
            self.assertEqual(loaded["source_skill"], "media_discovery")
            self.assertEqual(loaded["reason_code"], "challenge_required")
            self.assertTrue(loaded["browser_action"])
            self.assertEqual(
                Path(loaded["action_path"]).name,
                f"action-{loaded['handoff_id']}.json",
            )

    def test_browser_handoff_action_is_private_and_bound_to_current_challenge(self):
        with tempfile.TemporaryDirectory() as directory:
            challenge_dir = Path(directory) / "media_discovery" / "login_challenge"
            challenge_dir.mkdir(parents=True)
            handoff_id = "12345678-1234-1234-1234-123456789abc"
            action_path = challenge_dir / f"action-{handoff_id}.json"
            challenge = {
                "browser_action": True,
                "handoff_id": handoff_id,
                "action_path": str(action_path),
            }
            self.assertTrue(
                request_login_challenge_browser(
                    challenge,
                    now=123.5,
                )
            )
            action = json.loads(action_path.read_text(encoding="utf-8"))
            self.assertEqual(action["schema_version"], 1)
            self.assertEqual(action["action"], "show_browser")
            self.assertEqual(action["handoff_id"], challenge["handoff_id"])
            self.assertEqual(action["requested_at_epoch"], 123.5)
            self.assertEqual(action_path.stat().st_mode & 0o777, 0o600)
            self.assertFalse(
                request_login_challenge_browser(
                    {
                        "browser_action": True,
                        "handoff_id": "../unsafe",
                        "action_path": str(action_path),
                    },
                )
            )


if __name__ == "__main__":
    unittest.main()
