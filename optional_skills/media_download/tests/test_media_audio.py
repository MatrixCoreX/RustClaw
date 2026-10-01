import importlib.util
from pathlib import Path
import unittest
from unittest import mock


TOOL_ROOT = Path(__file__).parents[1] / "src" / "tool"
MODULE_PATH = TOOL_ROOT / "media_audio.py"


def load_media_audio_module():
    spec = importlib.util.spec_from_file_location("media_download_media_audio", MODULE_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class MediaAudioTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.audio = load_media_audio_module()

    def test_extract_command_is_deterministic(self) -> None:
        command = self.audio.build_extract_audio_command(
            "ffmpeg",
            Path("clip.mp4"),
            Path("clip.wav"),
            overwrite=True,
            sample_rate=16000,
            channels=1,
        )

        self.assertEqual(
            command,
            [
                "ffmpeg",
                "-hide_banner",
                "-y",
                "-i",
                "clip.mp4",
                "-map",
                "0:a:0",
                "-vn",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "clip.wav",
            ],
        )

    def test_probe_returns_none_without_ffprobe(self) -> None:
        with mock.patch.object(self.audio.shutil, "which", return_value=None):
            self.assertIsNone(self.audio.probe_audio_stream(Path("clip.mp4")))

    def test_probe_reports_audio_stream(self) -> None:
        completed = mock.Mock(returncode=0, stdout="1\n")
        with mock.patch.object(self.audio.shutil, "which", return_value="/usr/bin/ffprobe"), mock.patch.object(
            self.audio.subprocess,
            "run",
            return_value=completed,
        ) as run:
            self.assertTrue(self.audio.probe_audio_stream(Path("clip.mp4")))

        command = run.call_args.args[0]
        self.assertIn("-select_streams", command)
        self.assertIn("a:0", command)


if __name__ == "__main__":
    unittest.main()
