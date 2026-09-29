import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "media_downloader.py"


def load_module():
    spec = importlib.util.spec_from_file_location("media_download_youtube_tool", ENTRYPOINT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class YoutubeDownloadTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.tool = load_module()

    def test_login_challenge_retries_with_skill_owned_profile(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output"
            profile_root = root / "private-browser"
            saved = output / "video.mp4"
            challenge = subprocess.CompletedProcess(
                ["yt-dlp"],
                1,
                "",
                "ERROR: Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies",
            )
            success = subprocess.CompletedProcess(["yt-dlp"], 0, f"{saved}\n", "")

            with (
                mock.patch.object(self.tool, "find_yt_dlp_binary", return_value="/venv/yt-dlp"),
                mock.patch.object(self.tool, "find_ytdlp_js_runtime", return_value="node:/usr/bin/node"),
                mock.patch.object(self.tool, "find_chrome_executable", return_value="/usr/bin/chromium"),
                mock.patch.object(self.tool, "run_task_subprocess", side_effect=[challenge, success]) as runner,
                mock.patch.object(self.tool.youtube_access, "profile_has_cookies", return_value=False),
                mock.patch.object(self.tool.youtube_access, "wait_for_skill_owned_login", return_value="ok") as login,
            ):
                result = self.tool.download_youtube_video(
                    self.tool.Candidate("https://www.youtube.com/watch?v=dQw4w9WgXcQ", "test", 1),
                    output,
                    browser_profile_dir=str(profile_root),
                )

        self.assertEqual(result, saved)
        self.assertEqual(runner.call_count, 2)
        retry_command = runner.call_args_list[1].args[0]
        self.assertIn("--cookies-from-browser", retry_command)
        self.assertEqual(
            retry_command[retry_command.index("--js-runtimes") + 1],
            "node:/usr/bin/node",
        )
        browser_spec = retry_command[retry_command.index("--cookies-from-browser") + 1]
        self.assertTrue(browser_spec.startswith("chromium+basictext:"))
        login.assert_called_once()

    def test_interactive_login_failure_returns_structured_token(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            challenge = subprocess.CompletedProcess(
                ["yt-dlp"],
                1,
                "",
                "ERROR: Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies",
            )
            with (
                mock.patch.object(self.tool, "find_yt_dlp_binary", return_value="/venv/yt-dlp"),
                mock.patch.object(self.tool, "find_ytdlp_js_runtime", return_value=None),
                mock.patch.object(self.tool, "find_chrome_executable", return_value="/usr/bin/chromium"),
                mock.patch.object(self.tool, "run_task_subprocess", return_value=challenge),
                mock.patch.object(self.tool.youtube_access, "profile_has_cookies", return_value=False),
                mock.patch.object(
                    self.tool.youtube_access,
                    "wait_for_skill_owned_login",
                    return_value="display_unavailable",
                ),
            ):
                with self.assertRaisesRegex(self.tool.DouyinDownloadError, "display_unavailable"):
                    self.tool.download_youtube_video(
                        self.tool.Candidate(
                            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
                            "test",
                            1,
                        ),
                        Path(directory) / "output",
                        browser_profile_dir=str(Path(directory) / "private-browser"),
                    )

    @mock.patch("shutil.which")
    def test_js_runtime_prefers_supported_local_runtime(self, which) -> None:
        which.side_effect = lambda name: "/usr/bin/node" if name == "node" else None

        self.assertEqual(self.tool.find_ytdlp_js_runtime(), "node:/usr/bin/node")

    def test_youtube_command_omits_js_runtime_when_unavailable(self) -> None:
        command = self.tool.build_youtube_command(
            "/venv/yt-dlp",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            output_template=Path("/tmp/video.%(ext)s"),
        )

        self.assertNotIn("--js-runtimes", command)


if __name__ == "__main__":
    unittest.main()
