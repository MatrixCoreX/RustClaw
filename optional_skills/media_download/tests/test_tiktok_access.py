import importlib.util
from pathlib import Path
import json
import sys
import tempfile
import unittest
from unittest import mock


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "tiktok_access.py"


def load_module():
    spec = importlib.util.spec_from_file_location("media_download_tiktok_access", ENTRYPOINT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


def universal(detail):
    payload = json.dumps({"__DEFAULT_SCOPE__": {"webapp.video-detail": detail}})
    return f'<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{payload}</script>'


class TiktokAccessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.access = load_module()

    def test_private_profile_is_scoped_to_tiktok(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            profile = self.access.persistent_profile_dir(directory)
            self.assertEqual(profile, Path(directory) / "tiktok")
            self.assertTrue(profile.is_dir())

    def test_playable_item_is_available(self) -> None:
        page = universal({"itemInfo": {"itemStruct": {"id": "123", "video": {}}}})
        self.assertEqual(
            self.access.classify_page(page, "https://www.tiktok.com/@name/video/123", "123"),
            self.access.CONTENT_AVAILABLE,
        )

    def test_nonzero_video_status_is_unavailable_not_login(self) -> None:
        page = universal(
            {
                "statusCode": 10204,
                "statusMsg": "author_status,content_classification,status_audit_not_pass",
            }
        )
        self.assertEqual(
            self.access.classify_page(page, "https://www.tiktok.com/@name/video/123", "123"),
            self.access.CONTENT_UNAVAILABLE,
        )
        self.assertFalse(
            self.access.needs_skill_owned_login(
                page_text=page,
                url="https://www.tiktok.com/@name/video/123",
                expected_item_id="123",
                has_media=False,
                content_unavailable=True,
            )
        )

    def test_login_route_and_modal_are_login_barriers(self) -> None:
        self.assertEqual(
            self.access.classify_page("", "https://www.tiktok.com/login/qrcode"),
            self.access.LOGIN_REQUIRED,
        )
        modal = '<div data-e2e="login-modal"><canvas class="qrcode"></canvas></div>'
        self.assertTrue(
            self.access.needs_skill_owned_login(
                page_text=modal,
                url="https://www.tiktok.com/@name/video/123",
                expected_item_id="123",
                has_media=False,
            )
        )

    def test_normal_header_login_button_does_not_trigger_login(self) -> None:
        page = '<button data-e2e="top-login-button">Log in</button>'
        self.assertEqual(
            self.access.classify_page(page, "https://www.tiktok.com/@name/video/123", "123"),
            self.access.ACCESS_UNKNOWN,
        )

    def test_login_url_preserves_target_as_redirect(self) -> None:
        url = self.access._login_url("https://www.tiktok.com/@name/video/123?a=1")
        self.assertTrue(url.startswith("https://www.tiktok.com/login/qrcode?"))
        self.assertIn("redirect_url=", url)

    def test_unavailable_and_failed_interactive_outcomes_are_terminal(self) -> None:
        self.assertTrue(
            self.access.access_outcome_is_terminal(["tiktok: content_unavailable"])
        )
        self.assertTrue(
            self.access.access_outcome_is_terminal(
                ["tiktok: interactive_login=interactive_verification_timeout"]
            )
        )
        self.assertFalse(
            self.access.access_outcome_is_terminal(["tiktok: interactive_login=ok"])
        )

    def test_interactive_login_publishes_tiktok_challenge_for_pi_companion(self) -> None:
        class FakeProcess:
            def __init__(self) -> None:
                self.returncode = None

            def poll(self):
                return self.returncode

            def terminate(self):
                self.returncode = 0

            def wait(self, timeout=None):
                del timeout
                return self.returncode

            def kill(self):
                self.returncode = -9

        class FakeClient:
            def __init__(self) -> None:
                self.next_id = 0
                self.responses = []

            def send(self, method, params=None):
                del params
                self.next_id += 1
                result = (
                    {"cookies": [{"name": "sessionid"}]}
                    if method == "Network.getAllCookies"
                    else {}
                )
                self.responses.append({"id": self.next_id, "result": result})
                return self.next_id

            def recv(self, timeout):
                del timeout
                return self.responses.pop(0)

            def close(self):
                return None

        fake_process = FakeProcess()
        fake_client = FakeClient()
        with (
            tempfile.TemporaryDirectory() as directory,
            mock.patch.object(self.access, "desktop_display_available", return_value=True),
            mock.patch.object(self.access, "_devtools_port", return_value=(43123, [])),
            mock.patch.object(self.access, "_wait_for_devtools", return_value="ws://127.0.0.1/page"),
            mock.patch.object(self.access.subprocess, "Popen", return_value=fake_process),
            mock.patch.object(self.access, "DevToolsConnection", return_value=fake_client),
            mock.patch.object(self.access.login_challenge, "effective_scan_timeout", return_value=10.0),
            mock.patch.object(self.access.login_challenge, "challenge_has_expired", return_value=False),
            mock.patch.object(self.access.login_challenge, "publish_login_challenge", return_value=True) as publish,
            mock.patch.object(self.access.login_challenge, "clear_login_challenge") as clear,
            mock.patch.object(self.access.time, "sleep"),
        ):
            status = self.access.wait_for_skill_owned_login(
                chrome="chromium",
                profile_dir=Path(directory) / "browser-profile" / "tiktok",
                page_url="https://www.tiktok.com/@name/video/123",
            )

        self.assertEqual(status, "ok")
        self.assertEqual(publish.call_args.args[3], "tiktok")
        clear.assert_called_once_with(
            Path(directory) / "browser-profile" / "tiktok",
            "tiktok",
        )


if __name__ == "__main__":
    unittest.main()
