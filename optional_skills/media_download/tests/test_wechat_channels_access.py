import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "wechat_channels_access.py"


def load_module():
    spec = importlib.util.spec_from_file_location(
        "media_download_wechat_channels_access",
        ENTRYPOINT,
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class WechatChannelsAccessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.access = load_module()

    def test_extracts_and_normalizes_sph_share_url(self) -> None:
        text = "微信视频号 https://weixin.qq.com/sph/A88dF8Ju34#fragment 请下载"
        self.assertEqual(
            self.access.extract_share_url(text),
            "https://weixin.qq.com/sph/A88dF8Ju34",
        )
        self.assertIsNone(
            self.access.extract_share_url("https://mp.weixin.qq.com/s/example")
        )

    def test_rejects_unsafe_share_and_preview_urls(self) -> None:
        invalid_shares = (
            "http://weixin.qq.com/sph/A88dF8Ju34",
            "https://example.com/sph/A88dF8Ju34",
            "https://weixin.qq.com/sph/a/b",
            "https://weixin.qq.com:8443/sph/A88dF8Ju34",
        )
        for url in invalid_shares:
            with self.subTest(url=url):
                with self.assertRaises(self.access.WechatChannelsAccessError):
                    self.access.normalize_share_url(url)

        with self.assertRaises(self.access.WechatChannelsAccessError):
            self.access.parse_preview_url(
                "https://channels.weixin.qq.com/finder-preview/pages/feed"
                "?token=one&token=two&eid=item"
            )

    def test_private_profile_stays_under_skill_storage(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = self.access.persistent_profile_dir(root)
            assert profile is not None
            self.assertEqual(profile, root / "wechat_channels")
            self.assertEqual(profile.stat().st_mode & 0o777, 0o700)

    def test_feed_prefers_h264_and_preserves_article_and_images(self) -> None:
        playable = (
            "https://channels.weixin.qq.com/finder-preview/pages/feed"
            "?token=token-value&eid=export-value"
        )
        feed = {
            "errCode": 0,
            "errMsg": "",
            "data": {
                "authorInfo": {"nickname": "作者"},
                "feedInfo": {
                    "description": "正文",
                    "h264VideoInfo": {"videoUrl": "https://finder.video.qq.com/h264"},
                    "h265VideoInfo": {"videoUrl": "https://finder.video.qq.com/h265"},
                    "videoUrl": "https://finder.video.qq.com/fallback",
                    "coverUrl": "https://finder.video.qq.com/cover",
                },
            },
        }
        with mock.patch.object(self.access, "_post_json", return_value=(201, feed)):
            author, description, videos, images, cover = self.access.fetch_feed_info(
                playable,
                timeout=5,
            )

        self.assertEqual(author, "作者")
        self.assertEqual(description, "正文")
        self.assertEqual(videos[0][0], "h264VideoInfo.videoUrl")
        self.assertEqual(len(videos), 3)
        self.assertEqual(images, ())
        self.assertEqual(cover, "https://finder.video.qq.com/cover")

    def test_feed_ignores_malformed_nested_video_fields(self) -> None:
        playable = (
            "https://channels.weixin.qq.com/finder-preview/pages/feed"
            "?token=token-value&eid=export-value"
        )
        feed = {
            "errCode": 0,
            "data": {
                "feedInfo": {
                    "h264VideoInfo": "unexpected",
                    "h265VideoInfo": None,
                    "videoUrl": "https://finder.video.qq.com/fallback",
                }
            },
        }
        with mock.patch.object(self.access, "_post_json", return_value=(201, feed)):
            _, _, videos, images, _ = self.access.fetch_feed_info(
                playable,
                timeout=5,
            )

        self.assertEqual(
            videos,
            (("videoUrl", "https://finder.video.qq.com/fallback"),),
        )
        self.assertEqual(images, ())

    def test_feed_uses_ordered_images_when_video_is_absent(self) -> None:
        playable = (
            "https://channels.weixin.qq.com/finder-preview/pages/feed"
            "?token=token-value&eid=export-value"
        )
        feed = {
            "errCode": 0,
            "data": {
                "authorInfo": {"nickname": "作者"},
                "feedInfo": {
                    "description": "图文正文",
                    "picInfo": [
                        {"url": "https://finder.video.qq.com/1.jpg"},
                        {"url": "https://finder.video.qq.com/2.jpg"},
                    ],
                },
            },
        }
        with mock.patch.object(self.access, "_post_json", return_value=(200, feed)):
            _, _, videos, images, _ = self.access.fetch_feed_info(playable, timeout=5)

        self.assertEqual(videos, ())
        self.assertEqual(
            images,
            (
                "https://finder.video.qq.com/1.jpg",
                "https://finder.video.qq.com/2.jpg",
            ),
        )

    def test_missing_login_opens_visible_profile_and_resumes_same_request(self) -> None:
        playable = (
            "https://channels.weixin.qq.com/finder-preview/pages/feed"
            "?token=token-value&eid=export-value"
        )
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(
            self.access,
            "parse_share_with_browser",
            side_effect=[
                self.access.WechatChannelsAccessError(self.access.LOGIN_REQUIRED),
                {
                    "playable_url": playable,
                    "author": "fallback-author",
                    "desc": "fallback-description",
                },
            ],
        ) as browser_parse, mock.patch.object(
            self.access.xiaohongshu_access,
            "desktop_session_environment",
            return_value={"DISPLAY": ":0"},
        ), mock.patch.object(
            self.access.xiaohongshu_access,
            "desktop_display_available",
            return_value=True,
        ), mock.patch.object(
            self.access,
            "fetch_feed_info",
            return_value=("author", "description", (("videoUrl", "https://finder.video.qq.com/v"),), (), None),
        ):
            result = self.access.resolve_share(
                "https://weixin.qq.com/sph/A88dF8Ju34",
                browser_profile_root=directory,
                chrome="/usr/bin/google-chrome",
                timeout=5,
                browser_fallback=True,
            )

        self.assertEqual(result.export_id, "export-value")
        self.assertEqual(result.author, "author")
        self.assertEqual(browser_parse.call_count, 2)
        self.assertFalse(browser_parse.call_args_list[0].kwargs["visible"])
        self.assertTrue(browser_parse.call_args_list[1].kwargs["visible"])


if __name__ == "__main__":
    unittest.main()
