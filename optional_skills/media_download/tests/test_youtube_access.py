import importlib.util
from collections import deque
from pathlib import Path
import sys
import tempfile
import unittest


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "youtube_access.py"


def load_module():
    spec = importlib.util.spec_from_file_location("media_download_youtube_access", ENTRYPOINT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class YoutubeAccessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.access = load_module()

    def test_private_profile_stays_under_skill_storage(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = self.access.persistent_profile_dir(root)

            self.assertEqual(profile, root / "youtube")
            self.assertTrue(profile.is_dir())

    def test_ytdlp_browser_spec_uses_private_default_profile(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "youtube"

            linux = self.access.ytdlp_browser_spec(
                "/usr/bin/chromium",
                profile,
                host_platform="linux",
            )
            macos = self.access.ytdlp_browser_spec(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                profile,
                host_platform="darwin",
            )

        self.assertEqual(linux, f"chromium+basictext:{profile / 'Default'}")
        self.assertEqual(macos, f"chrome:{profile / 'Default'}")

    def test_login_diagnostic_is_provider_scoped(self) -> None:
        self.assertTrue(
            self.access.requires_login_diagnostic(
                "ERROR: Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies"
            )
        )
        self.assertFalse(self.access.requires_login_diagnostic("HTTP Error 503: temporarily unavailable"))

    def test_linux_requires_a_display_for_interactive_login(self) -> None:
        self.assertFalse(
            self.access.desktop_display_available({}, host_platform="linux")
        )
        self.assertTrue(
            self.access.desktop_display_available(
                {"WAYLAND_DISPLAY": "wayland-0"},
                host_platform="linux",
            )
        )

    def test_login_observation_reads_each_response_before_next_command(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.calls = []
                self.responses = deque()
                self.next_id = 0

            def send(self, method, params=None):
                del params
                if self.responses:
                    raise AssertionError(f"sent {method} before reading the prior response")
                self.next_id += 1
                self.calls.append(method)
                if method == "Runtime.evaluate":
                    result = {
                        "result": {
                            "value": {
                                "host": "www.youtube.com",
                                "has_player": True,
                                "ready_state": "complete",
                            }
                        }
                    }
                else:
                    result = {"cookies": [{"name": "SID"}]}
                self.responses.append({"id": self.next_id, "result": result})
                return self.next_id

            def recv(self, timeout):
                del timeout
                return self.responses.popleft()

        client = FakeClient()
        state, cookies = self.access._read_login_observation(client, timeout=1.0)

        self.assertEqual(client.calls, ["Runtime.evaluate", "Network.getAllCookies"])
        self.assertEqual(state["host"], "www.youtube.com")
        self.assertEqual(cookies, {"SID"})


if __name__ == "__main__":
    unittest.main()
