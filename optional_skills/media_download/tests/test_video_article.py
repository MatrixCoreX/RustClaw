import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock

TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"


def load_tool():
    spec = importlib.util.spec_from_file_location("video_article_tool", TOOL_DIR / "media_downloader.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class VideoArticleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tool = load_tool()

    def test_download_collects_structured_metadata_without_changing_resolve(self):
        command = self.tool.build_youtube_command(
            "yt-dlp", "https://www.youtube.com/watch?v=example", output_template=Path("post.%(ext)s")
        )
        self.assertIn("--write-info-json", command)
        resolve = self.tool.build_youtube_command(
            "yt-dlp", "https://www.youtube.com/watch?v=example", print_url=True
        )
        self.assertNotIn("--write-info-json", resolve)

    def test_ytdlp_description_is_delivered_with_video_for_both_adapters(self):
        for platform in ("youtube", "bilibili"):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                video = root / "post.mp4"
                video.write_bytes(b"video")
                video.with_suffix(".info.json").write_text(json.dumps({
                    "title": "Source title", "description": "First line.\n原始正文 &amp; emoji 🎬",
                    "uploader": "Source author",
                }), encoding="utf-8")
                args = self.tool.parse_args(["--output-dir", directory, "--no-browser-fallback", "https://example.test/post"])
                stdout = io.StringIO()
                with (
                    mock.patch.object(self.tool, "download_youtube_video", return_value=video),
                    redirect_stdout(stdout), redirect_stderr(io.StringIO()),
                ):
                    status = self.tool.handle_resolved_media(
                        args, "https://example.test/post", None, platform, "source-id",
                        [self.tool.Candidate("https://example.test/video", "test", 1)], [], [],
                    )
                article = root / "post_article.txt"
                self.assertEqual(status, 0)
                self.assertEqual(stdout.getvalue().splitlines(), [str(article), str(video)])
                self.assertIn("First line.\n原始正文 & emoji 🎬", article.read_text())
                self.assertIn("作者：Source author", article.read_text())

    def test_missing_or_invalid_description_never_uses_title_as_body(self):
        invalid = [None, {}, {"title": "Only a title"}, {"description": " \n "}, {"description": ["not prose"]}]
        with tempfile.TemporaryDirectory() as directory:
            video = Path(directory) / "post.mp4"
            for payload in invalid:
                with self.subTest(payload=payload):
                    video.with_suffix(".info.json").write_text(json.dumps(payload))
                    self.assertIsNone(self.tool.article_from_ytdlp_info(video))
            video.with_suffix(".info.json").write_text("invalid json")
            self.assertIsNone(self.tool.article_from_ytdlp_info(video))
            video.with_suffix(".info.json").unlink()
            self.assertIsNone(self.tool.article_from_ytdlp_info(video))
            title_only = self.tool.ArticleContent("Only a title", "", "Author", "source")
            self.assertIsNone(self.tool.save_video_article_if_available(
                title_only, platform="douyin", output_dir=Path(directory), output_name="post.mp4",
                item_id=None, share_text="User-supplied share copy is not article prose", overwrite=False,
            ))
            self.assertFalse((Path(directory) / "post_article.txt").exists())

    def test_structured_video_caption_is_saved_for_douyin_and_xiaohongshu(self):
        for platform in ("douyin", "xiaohongshu"):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                args = self.tool.parse_args(["--output-dir", directory, "--output-name", "post", "--no-browser-fallback", "https://example.test/post"])
                article = self.tool.ArticleContent("Post title", "Platform caption body.", "Author", "structured-payload")

                def download(_candidate, output_path, **_kwargs):
                    output_path.write_bytes(b"video")
                    return output_path

                with (
                    mock.patch.object(self.tool, "download_candidate", side_effect=download),
                    mock.patch.object(self.tool.media_audio, "probe_audio_stream", return_value=True),
                    redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()),
                ):
                    self.assertEqual(self.tool.handle_resolved_media(
                        args, "https://example.test/post", None, platform, "source-id",
                        [self.tool.Candidate("https://example.test/video", "test", 1)], [], [], article=article,
                    ), 0)
                self.assertIn("Platform caption body.", (root / "post_article.txt").read_text())

    def test_metadata_files_cannot_be_mistaken_for_downloaded_video(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            video = root / "post.mp4"
            video.write_bytes(b"video")
            (root / "post.info.json").write_text('{"description":"Source body"}')
            (root / "post.description").write_text("Source body")
            self.assertEqual(self.tool.find_downloaded_youtube_output(root / "post.%(ext)s"), video)

    def test_optional_article_write_failure_keeps_downloaded_video(self):
        with tempfile.TemporaryDirectory() as directory:
            article = self.tool.ArticleContent("Post title", "Source body", "Author", "source")
            with mock.patch.object(self.tool, "save_article_content", side_effect=OSError("disk full")), redirect_stderr(io.StringIO()):
                self.assertIsNone(self.tool.save_video_article_if_available(
                    article, platform="douyin", output_dir=Path(directory), output_name="post.mp4",
                    item_id=None, share_text="https://example.test/post", overwrite=False,
                ))


if __name__ == "__main__":
    unittest.main()
