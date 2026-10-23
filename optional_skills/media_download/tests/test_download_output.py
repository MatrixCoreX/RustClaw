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


class OriginalMediaOutputTests(unittest.TestCase):
    def test_automatic_media_selects_originals_and_audio_only_on_request(self):
        video = {"mime_type": "video/mp4", "path": "/unused.mp4"}
        audio = {"mime_type": "audio/mp4", "path": "/unused.m4a"}
        archive = {"mime_type": "application/zip", "artifact_role": "image_archive", "path": "/unused.zip"}
        self.assertEqual(m.select_download_output([video, audio], "media", None), [video])
        self.assertEqual(m.select_download_output([archive, audio], "media", None), [archive])
        self.assertEqual(m.select_download_output([archive, audio], "media_audio", None), [archive, audio])
        self.assertEqual(m.select_download_output([archive], "media_audio", None), [archive])
        article = {"mime_type": "text/plain", "artifact_role": "article_text", "path": "/original.txt"}
        self.assertEqual(m.select_download_output([video, audio, article], "media", None), [video, article])
        self.assertEqual(m.select_download_output([archive, audio, article], "media_audio", None), [archive, audio, article])
        self.assertEqual(m.select_download_output([article], "media", None), [article])

    def test_explicit_zero_can_archive_a_single_image_without_changing_legacy_threshold(self):
        import zipfile
        main_spec = importlib.util.spec_from_file_location("download_archive_fixture", MODULE.parent.parent / "main.py")
        main = importlib.util.module_from_spec(main_spec)
        main_spec.loader.exec_module(main)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            image = root / "001.jpg"
            image.write_bytes(b"original-image")
            artifacts = [dict(main._artifact(image), artifact_role="original_image")]
            legacy, delivery, inputs = main._package_large_image_delivery(artifacts, root)
            self.assertEqual(legacy, artifacts)
            self.assertIsNone(delivery)
            packed, delivery, inputs = main._package_large_image_delivery(artifacts, root, image_archive_threshold=0)
            self.assertTrue(image.exists())
            self.assertEqual(inputs["image_count"], 1)
            self.assertEqual(delivery["image_count"], 1)
            with zipfile.ZipFile(packed[0]["path"]) as archive:
                self.assertEqual(archive.namelist(), ["001.jpg"])
                self.assertEqual(archive.read("001.jpg"), b"original-image")
