import importlib.util
from pathlib import Path
import sys
import unittest


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"


def load_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, TOOL_DIR / filename)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class XiaohongshuAccessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.access = load_module("media_download_xiaohongshu_access", "xiaohongshu_access.py")
        cls.downloader = load_module("media_download_share_text_tool_xhs", "media_downloader.py")

    def test_recovers_note_and_token_from_login_redirect(self) -> None:
        login = (
            "https://www.xiaohongshu.com/login?redirectPath="
            "http%3A%2F%2Fwww.xiaohongshu.com%2Fdiscovery%2Fitem%2F6aafd2c5000000000d027a00"
            "%3Fxsec_token%3Dexample-token%26type%3Dvideo"
        )
        target = self.access.recover_from_login_url(login)
        self.assertIsNotNone(target)
        assert target is not None
        self.assertEqual(target.note_id, "6aafd2c5000000000d027a00")
        self.assertEqual(target.xsec_token, "example-token")
        self.assertTrue(target.login_barrier)
        self.assertIn("/discovery/item/6aafd2c5000000000d027a00", target.page_url)
        self.assertIn("xsec_token=example-token", target.page_url)

    def test_prefers_note_page_over_login_bounce(self) -> None:
        urls = [
            "https://xhslink.cn/o/example",
            "https://www.xiaohongshu.com/login?redirectPath="
            "https%3A%2F%2Fwww.xiaohongshu.com%2Fexplore%2F6aafd2c5000000000d027a00"
            "%3Fxsec_token%3Dkeep-me",
        ]
        preferred = self.access.preferred_browser_urls(urls[0], urls)
        self.assertEqual(preferred[0].split("?", 1)[0], "https://www.xiaohongshu.com/discovery/item/6aafd2c5000000000d027a00")
        self.assertTrue(all("/login" not in url for url in preferred))

    def test_accepts_ef_stream_master_urls(self) -> None:
        payload = {
            "video": {
                "media": {
                    "stream": {
                        "EF4": [
                            {
                                "masterUrl": "https://sns-video-v3.xhscdn.com/note/ef4/master",
                                "backupUrls": ["https://sns-bak-v1.xhscdn.com/note/ef4/backup"],
                            }
                        ],
                        "h264": [],
                    }
                }
            }
        }
        self.assertTrue(self.access.looks_like_xiaohongshu_video_url(payload["video"]["media"]["stream"]["EF4"][0]["masterUrl"]))
        candidates = self.downloader.extract_xiaohongshu_candidates_from_json(payload)
        urls = {candidate.url for candidate in candidates}
        self.assertIn("https://sns-video-v3.xhscdn.com/note/ef4/master", urls)
        self.assertIn("https://sns-bak-v1.xhscdn.com/note/ef4/backup", urls)

    def test_login_html_without_note_state_is_a_barrier(self) -> None:
        html = '<html><script>window.__INITIAL_STATE__={"user":{"loggedIn":false}}</script></html>'
        self.assertTrue(
            self.access.xiaohongshu_html_is_login(
                html,
                "https://www.xiaohongshu.com/login?redirectPath=%2Fexplore%2Fabc",
            )
        )
        self.assertFalse(
            self.access.xiaohongshu_html_is_login(
                '<script>window.__INITIAL_STATE__={"note":{"noteDetailMap":{"abc":{"note":{"noteId":"abc"}}}}}</script>',
                "https://www.xiaohongshu.com/explore/abc",
            )
        )

    def test_empty_or_timed_out_dump_after_http_login_needs_skill_owned_login(self) -> None:
        item_url = "https://www.xiaohongshu.com/discovery/item/6aafd2c5000000000d027a00?xsec_token=keep"
        self.assertFalse(self.access.xiaohongshu_html_is_login("", item_url))
        self.assertTrue(
            self.access.xiaohongshu_needs_skill_owned_login(
                page_text="",
                url=item_url,
                http_login_barrier=True,
                dump_timed_out=True,
                has_media=False,
            )
        )
        self.assertFalse(
            self.access.xiaohongshu_needs_skill_owned_login(
                page_text="",
                url=item_url,
                http_login_barrier=True,
                dump_timed_out=True,
                has_media=True,
            )
        )
        self.assertTrue(
            self.access.xiaohongshu_login_outcome_is_terminal(
                ["xiaohongshu: login_required", "parse_attempt: 1/4"]
            )
        )

    def test_note_access_ready_requires_requested_id(self) -> None:
        self.assertFalse(self.access.note_access_ready({"login": True, "note_ids": ["abc"]}, "abc"))
        self.assertFalse(self.access.note_access_ready({"login": False, "note_ids": ["other"]}, "abc"))
        self.assertTrue(self.access.note_access_ready({"login": False, "note_ids": ["abc"]}, "abc"))

    def test_desktop_display_detection(self) -> None:
        self.assertTrue(self.access.desktop_display_available({"DISPLAY": ":0"}, "linux"))
        self.assertFalse(self.access.desktop_display_available({}, "linux"))
        self.assertTrue(self.access.desktop_display_available({}, "darwin"))
        self.assertEqual(
            self.access.visible_chrome_args({"WAYLAND_DISPLAY": "wayland-0"}, "linux"),
            ["--ozone-platform=wayland"],
        )
        self.assertEqual(self.access.visible_chrome_args({"DISPLAY": ":0"}, "linux"), [])

    def test_manifest_forwards_host_display_variables(self) -> None:
        manifest = (Path(__file__).parents[1] / "skill.toml").read_text(encoding="utf-8")
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR"):
            self.assertIn(f'"{key}"', manifest)


if __name__ == "__main__":
    unittest.main()
