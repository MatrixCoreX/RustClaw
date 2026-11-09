"""Retry real progress/transient failures, not identical exhausted fallbacks."""
from types import SimpleNamespace
import unittest
from unittest import mock

from test_share_text import load_downloader_module


class ParseRetryProgressTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tool = load_downloader_module()

    def run_results(self, observations):
        args = SimpleNamespace(platform="auto", timeout=1, browser_fallback=True,
                               browser_timeout=1, chrome_path=None,
                               system_browser_cookies=False, extract_audio=False,
                               transcribe=False, print_url=False)
        with mock.patch.object(self.tool, "gather_candidates_for_request",
                               side_effect=observations) as gather:
            result = self.tool.gather_candidates_for_request_with_retries(
                args, "https://v.douyin.com/fixture/", None)
        return result, gather.call_count

    def test_unchanged_empty_result_stops_without_reopening_four_browsers(self):
        empty = ("douyin", "fixture-id", [], [], None, [])
        result, count = self.run_results([empty] * 4)
        self.assertEqual(result, empty)
        self.assertEqual(count, 2)

    def test_new_resolved_identity_is_progress(self):
        unresolved = ("douyin", None, [], [], None, [])
        resolved = ("douyin", "fixture-id", [], [], None, [])
        result, count = self.run_results([unresolved, resolved, resolved])
        self.assertEqual(result, resolved)
        self.assertEqual(count, 3)

    def test_transient_error_can_recover(self):
        media = self.tool.Candidate("https://media.test/video.mp4", "fixture", 1)
        complete = ("douyin", "fixture-id", [media], [], None, [])
        result, count = self.run_results([TimeoutError("fixture"), complete])
        self.assertEqual(result, complete)
        self.assertEqual(count, 2)

    def test_empty_then_complete_does_not_abort_early(self):
        image = self.tool.ImageCandidate("https://media.test/1.webp", "fixture", 1)
        article = self.tool.ArticleContent("", "caption", "", "fixture")
        empty = ("douyin", "fixture-id", [], [], None, [])
        complete = ("douyin", "fixture-id", [], [image], article, [])
        result, count = self.run_results([empty, complete])
        self.assertEqual(result, complete)
        self.assertEqual(count, 2)

    def test_partial_media_is_retained_while_retrying_missing_caption(self):
        image = self.tool.ImageCandidate("https://media.test/1.webp", "fixture", 1)
        partial = ("douyin", "fixture-id", [], [image], None, [])
        result, count = self.run_results([partial] * 4)
        self.assertEqual(result, partial)
        self.assertEqual(count, 4)


if __name__ == "__main__":
    unittest.main()
