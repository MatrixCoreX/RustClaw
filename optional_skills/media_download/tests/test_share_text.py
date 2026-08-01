import importlib.util
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "media_downloader.py"


def load_downloader_module():
    spec = importlib.util.spec_from_file_location("media_download_share_text_tool", ENTRYPOINT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class ShareTextTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.downloader = load_downloader_module()

    def assert_share_text(self, text: str, expected_url: str, expected_platform: str) -> None:
        self.assertEqual(self.downloader.extract_urls(text), [expected_url])
        self.assertEqual(self.downloader.detect_platform(text), expected_platform)

    def test_douyin_phone_share_text(self) -> None:
        url = "https://v.douyin.com/s3eZp4vFHeU/"
        self.assert_share_text(
            "6.97 复制打开抖音，看看这个作品 " + url + " H@I.ic :1pm teb:/ 11/29",
            url,
            "douyin",
        )

    def test_douyin_website_video_and_note_urls(self) -> None:
        for url in (
            "https://www.douyin.com/video/7658893225607908651",
            "https://www.douyin.com/note/7658893225607908651?modal_id=7658893225607908651",
        ):
            with self.subTest(url=url):
                self.assert_share_text(url, url, "douyin")
                self.assertEqual(
                    self.downloader.extract_aweme_id(url),
                    "7658893225607908651",
                )

    def test_kuaishou_phone_share_text(self) -> None:
        url = "https://v.kuaishou.com/AbCdEf"
        self.assert_share_text(
            "复制这条消息，打开【快手】查看精彩视频 " + url + " 该作品值得一看",
            url,
            "kuaishou",
        )

    def test_kuaishou_website_share_url(self) -> None:
        url = "https://www.kuaishou.com/short-video/3xexample?photoId=3xexample"
        self.assert_share_text(url, url, "kuaishou")
        self.assertEqual(self.downloader.extract_kuaishou_id(url), "3xexample")

    def test_xiaohongshu_app_share_text_with_cn_short_link(self) -> None:
        url = "http://xhslink.cn/o/5H9ovWUGFUd"
        self.assert_share_text(
            "发现我妈是低智商儿的证据 " + url + " 复制一下这行字，然后打开【小红书】看笔记。",
            url,
            "xiaohongshu",
        )

    def test_xiaohongshu_app_share_text_with_com_short_link(self) -> None:
        url = "https://xhslink.com/a/AbCdEf"
        self.assert_share_text(
            "我在小红书发现了一篇笔记，快来看看吧！" + url + " 复制本条信息打开小红书",
            url,
            "xiaohongshu",
        )

    def test_xiaohongshu_web_share_url(self) -> None:
        url = (
            "https://www.xiaohongshu.com/explore/66a1b2c3d4e5f60718293abc"
            "?xsec_token=example-token&xsec_source=pc_user"
        )
        self.assert_share_text(url, url, "xiaohongshu")
        self.assertEqual(
            self.downloader.extract_xiaohongshu_id(url),
            "66a1b2c3d4e5f60718293abc",
        )

    def test_xiaohongshu_legacy_web_item_url(self) -> None:
        url = (
            "https://www.xiaohongshu.com/discovery/item/66a1b2c3d4e5f60718293abc"
            "?source=webshare&xsec_source=pc_share"
        )
        self.assert_share_text(url, url, "xiaohongshu")
        self.assertEqual(
            self.downloader.extract_xiaohongshu_id(url),
            "66a1b2c3d4e5f60718293abc",
        )

    def test_tiktok_app_and_website_share_urls(self) -> None:
        cases = (
            ("https://vm.tiktok.com/ZMexample/", None),
            (
                "https://www.tiktok.com/@bamperboll_tm/video/7652359434898787592"
                "?is_from_webapp=1&sender_device=pc",
                "7652359434898787592",
            ),
        )
        for url, expected_id in cases:
            with self.subTest(url=url):
                self.assert_share_text(
                    "Shared from TikTok " + url,
                    url,
                    "tiktok",
                )
                if expected_id is not None:
                    self.assertEqual(
                        self.downloader.extract_tiktok_id(url),
                        expected_id,
                    )

    def test_youtube_app_and_website_share_urls(self) -> None:
        cases = (
            ("https://youtu.be/dQw4w9WgXcQ?si=example", "dQw4w9WgXcQ"),
            (
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ&feature=shared",
                "dQw4w9WgXcQ",
            ),
            ("https://www.youtube.com/shorts/dQw4w9WgXcQ", "dQw4w9WgXcQ"),
        )
        for url, expected_id in cases:
            with self.subTest(url=url):
                self.assert_share_text(url, url, "youtube")
                self.assertEqual(
                    self.downloader.extract_youtube_id(url),
                    expected_id,
                    )

    def test_douyin_image_post_extracts_platform_article(self) -> None:
        payload = {
            "aweme_id": "7658893225607908651",
            "desc": "第一段\r\n第二段\u200b",
            "author": {"nickname": "图文作者"},
            "images": [{"url_list": ["https://p.test/1.jpg"]}],
        }

        article = self.downloader.article_from_douyin_payload(
            payload,
            source="test",
        )

        self.assertIsNotNone(article)
        assert article is not None
        self.assertEqual(article.body, "第一段\n第二段")
        self.assertEqual(article.author, "图文作者")

    def test_xiaohongshu_article_matches_requested_note_only(self) -> None:
        requested_id = "66a1b2c3d4e5f60718293abc"
        state = {
            "noteDetailMap": {
                requested_id: {
                    "note": {
                        "noteId": requested_id,
                        "title": "目标标题",
                        "desc": "目标正文",
                        "user": {"nickname": "目标作者"},
                        "imageList": [{"urlDefault": "https://sns-webpic-qc.xhscdn.com/target.jpg"}],
                    }
                },
                "other": {
                    "note": {
                        "noteId": "ffffffffffffffffffffffff",
                        "title": "推荐内容",
                        "desc": "这不是目标笔记",
                        "imageList": [{"urlDefault": "https://sns-webpic-qc.xhscdn.com/other.jpg"}],
                    }
                },
            }
        }

        payload = self.downloader.find_xiaohongshu_note_payload(state, requested_id)
        self.assertIsNotNone(payload)
        assert payload is not None
        article = self.downloader.article_from_xiaohongshu_payload(payload, source="test")
        self.assertIsNotNone(article)
        assert article is not None
        self.assertEqual(article.title, "目标标题")
        self.assertEqual(article.body, "目标正文")
        self.assertEqual(article.author, "目标作者")
        images = self.downloader.extract_xiaohongshu_item_image_candidates(
            payload,
            source="test",
        )
        self.assertEqual(
            [image.url for image in images],
            ["https://sns-webpic-qc.xhscdn.com/target.jpg"],
        )

    def test_xiaohongshu_initial_state_allows_javascript_undefined(self) -> None:
        requested_id = "6a6a1797000000000100fd40"
        page = (
            '<script>window.__INITIAL_STATE__={"global":{"channels":undefined},'
            f'"note":{{"noteDetailMap":{{"{requested_id}":{{"note":'
            f'{{"noteId":"{requested_id}","title":"化石标题","desc":"化石正文",'
            '"user":{"nickname":"收藏者"},"imageList":[{"url":"https://p.test/1.jpg"}]}'
            "}}}}}}}</script>"
        )

        payload = self.downloader.find_xiaohongshu_note_payload_in_html(page, requested_id)
        self.assertIsNotNone(payload)
        assert payload is not None
        article = self.downloader.article_from_xiaohongshu_payload(payload, source="test")
        self.assertIsNotNone(article)
        assert article is not None
        self.assertEqual((article.title, article.body, article.author), ("化石标题", "化石正文", "收藏者"))

    def test_image_article_download_saves_images_and_article_separately(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output_dir = Path(directory)
            image_paths = [output_dir / "note_01.jpg", output_dir / "note_02.jpg"]

            def fake_download_images(*_args, **_kwargs):
                for path in image_paths:
                    path.write_bytes(b"image")
                return image_paths

            args = SimpleNamespace(
                verbose=False,
                print_url=False,
                extract_audio=False,
                transcribe=False,
                browser_fallback=False,
                output_dir=str(output_dir),
                output_name="note",
                overwrite=True,
                timeout=1.0,
                save_meta=False,
                show_info=False,
                ocr_images=False,
            )
            article = self.downloader.ArticleContent(
                "目标标题",
                "目标正文第一段\n目标正文第二段",
                "目标作者",
                "test",
            )
            with mock.patch.object(
                self.downloader,
                "download_image_candidates",
                side_effect=fake_download_images,
            ):
                result = self.downloader.handle_resolved_media(
                    args,
                    "图文分享 https://www.xiaohongshu.com/explore/66a1b2c3d4e5f60718293abc",
                    None,
                    "xiaohongshu",
                    "66a1b2c3d4e5f60718293abc",
                    [],
                    [
                        self.downloader.ImageCandidate("https://p.test/1.jpg", "test", 1),
                        self.downloader.ImageCandidate("https://p.test/2.jpg", "test", 2),
                    ],
                    [],
                    article=article,
                )

            self.assertEqual(result, 0)
            article_path = output_dir / "note_article.txt"
            self.assertTrue(article_path.is_file())
            article_text = article_path.read_text(encoding="utf-8")
            self.assertIn("标题：目标标题", article_text)
            self.assertIn("作者：目标作者", article_text)
            self.assertIn("目标正文第一段\n目标正文第二段", article_text)
            self.assertEqual(len(list(output_dir.glob("note_*.jpg"))), 2)

    def test_video_download_prefers_candidate_with_audio(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output_dir = Path(directory)
            candidates = [
                self.downloader.Candidate("https://media.test/video-only", "first", 1),
                self.downloader.Candidate("https://media.test/with-audio", "second", 2),
            ]
            calls = []

            def fake_download(candidate, output_path, **_kwargs):
                calls.append(candidate.source)
                output_path.write_bytes(candidate.source.encode("utf-8"))
                return output_path

            args = SimpleNamespace(
                verbose=False,
                print_url=False,
                extract_audio=False,
                transcribe=False,
                browser_fallback=False,
                output_dir=str(output_dir),
                output_name="clip.mp4",
                overwrite=True,
                timeout=1.0,
                save_meta=False,
            )
            with (
                mock.patch.object(self.downloader, "download_candidate", side_effect=fake_download),
                mock.patch.object(
                    self.downloader.video_transcriber,
                    "probe_audio_stream",
                    side_effect=[False, True],
                ),
                mock.patch.object(self.downloader, "handle_downloaded_video") as handled,
            ):
                result = self.downloader.handle_resolved_media(
                    args,
                    "https://v.douyin.com/example/",
                    None,
                    "douyin",
                    "item-1",
                    candidates,
                    [],
                    [],
                )

            self.assertEqual(result, 0)
            self.assertEqual(calls, ["first", "second"])
            self.assertEqual((output_dir / "clip.mp4").read_bytes(), b"second")
            handled.assert_called_once_with(output_dir / "clip.mp4", args)

    def test_genuinely_silent_video_keeps_best_candidate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output_dir = Path(directory)
            candidates = [
                self.downloader.Candidate("https://media.test/best", "best", 1),
                self.downloader.Candidate("https://media.test/fallback", "fallback", 2),
            ]

            def fake_download(candidate, output_path, **_kwargs):
                output_path.write_bytes(candidate.source.encode("utf-8"))
                return output_path

            args = SimpleNamespace(
                verbose=False,
                print_url=False,
                extract_audio=False,
                transcribe=False,
                browser_fallback=False,
                output_dir=str(output_dir),
                output_name="silent.mp4",
                overwrite=True,
                timeout=1.0,
                save_meta=False,
            )
            with (
                mock.patch.object(self.downloader, "download_candidate", side_effect=fake_download),
                mock.patch.object(
                    self.downloader.video_transcriber,
                    "probe_audio_stream",
                    side_effect=[False, False],
                ),
                mock.patch.object(self.downloader, "handle_downloaded_video") as handled,
            ):
                result = self.downloader.handle_resolved_media(
                    args,
                    "https://v.douyin.com/silent/",
                    None,
                    "douyin",
                    "item-2",
                    candidates,
                    [],
                    [],
                )

            self.assertEqual(result, 0)
            self.assertEqual((output_dir / "silent.mp4").read_bytes(), b"best")
            handled.assert_called_once_with(output_dir / "silent.mp4", args)


if __name__ == "__main__":
    unittest.main()
