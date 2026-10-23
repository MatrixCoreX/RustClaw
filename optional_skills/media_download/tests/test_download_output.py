import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

MODULE = Path(__file__).parents[1] / "src/tool/download_output.py"
spec = importlib.util.spec_from_file_location("download_output", MODULE)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class DownloadOutputTests(unittest.TestCase):
    def test_omitted_mode_preserves_existing_post_components(self):
        artifacts = [{"mime_type": "image/jpeg"}, {"mime_type": "text/plain"}]
        self.assertIs(m.select_download_output(artifacts, None, None), artifacts)
        with self.assertRaises(m.OutputSelectionError):
            m.validate_output_mode("transcribe")

    @unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "requires ffmpeg")
    def test_real_audio_extraction_preserves_channels_and_sample_rate(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "input.mp4"
            subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "color=size=64x64:rate=5", "-f", "lavfi", "-i", "sine=sample_rate=48000", "-t", "0.4", "-ac", "2", "-c:v", "mpeg4", "-c:a", "aac", str(source)], check=True)
            artifacts = [{"mime_type": "video/mp4", "path": str(source)}]
            result = m.select_download_output(artifacts, "video_audio", lambda p: {"path": str(p), "size_bytes": p.stat().st_size})
            self.assertEqual(len(result), 2)
            probe = subprocess.check_output(["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries", "stream=sample_rate,channels", "-of", "csv=p=0", result[1]["path"]], text=True).strip()
            self.assertEqual(probe, "48000,2")
            self.assertEqual(result[1]["mime_type"], "audio/mp4")
            only = m.select_download_output(result, "audio", None)
            self.assertEqual(len(only), 1)
            self.assertEqual(only[0]["artifact_role"], "extracted_audio")

    def test_audio_missing_does_not_suppress_video(self):
        from unittest.mock import patch
        artifacts = [{"mime_type": "video/mp4", "path": str(Path(tempfile.gettempdir()) / "not-used.mp4")}]
        with patch.object(m.shutil, "which", return_value="ffmpeg"):
            result = m.select_download_output(artifacts, "video_audio", None, run=lambda *a, **k: subprocess.CompletedProcess(a, 1))
            self.assertEqual(result, artifacts)

    def test_public_jobs_use_a_separate_browser_profile(self):
        main_spec = importlib.util.spec_from_file_location("download_main_fixture", MODULE.parent.parent / "main.py")
        main = importlib.util.module_from_spec(main_spec)
        main_spec.loader.exec_module(main)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            command = main._build_download_command({"share": "https://www.douyin.com/video/123", "public_only": True}, root / "output", resolve_only=False, storage_directory=root / "private")
            profile = command[command.index("--browser-profile-dir") + 1]
            self.assertIn("public-browser-profile", profile)
            self.assertIn("--no-system-browser-cookies", command)
            private = main._build_download_command({"share": "https://www.douyin.com/video/123"}, root / "output", resolve_only=False, storage_directory=root / "private")
            self.assertNotEqual(profile, private[private.index("--browser-profile-dir") + 1])
