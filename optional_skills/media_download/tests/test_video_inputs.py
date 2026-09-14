import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import wave

ROOT = Path(__file__).parents[1]
sys.path.insert(0, str(ROOT / "src" / "tool"))
import video_inputs


def write_audio(path):
    with wave.open(str(path), "wb") as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(16000)
        audio.writeframes(b"\0\0" * 1600)


class VideoInputsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "video with spaces;$(literal).mp4"
        self.source.write_bytes(b"original video")
        self.artifacts = [{"artifact_role": "original_video", "path": str(self.source)}]
        spec = importlib.util.spec_from_file_location("media_video_inputs_test", ROOT / "src/main.py")
        self.skill = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.skill)

    def prepare(self, scope, existing=None, progress=None):
        return video_inputs.prepare_video_inputs(
            self.artifacts, existing, self.root, scope, self.skill._artifact, progress,
        )

    @staticmethod
    def fake_ffmpeg(command, **kwargs):
        output = Path(command[-1])
        if output.suffix == ".wav":
            write_audio(output)
        else:
            output.write_bytes(b"frame")
        return subprocess.CompletedProcess(command, 0, "", "")

    def test_audio_only_prepares_wav_with_no_recognizer_or_shell(self):
        progress = mock.Mock()
        with mock.patch.object(video_inputs.subprocess, "run", side_effect=self.fake_ffmpeg) as run:
            inputs = self.prepare("audio_only", progress=progress)
        command = run.call_args.args[0]
        self.assertEqual(run.call_count, 1)
        self.assertEqual(command[0], "ffmpeg")
        self.assertEqual(command[command.index("-i") + 1], str(self.source))
        self.assertIn("-vn", command)
        self.assertNotIn("shell", run.call_args.kwargs)
        self.assertNotIn("timeout", run.call_args.kwargs)
        self.assertNotIn("video_first_frame", inputs)
        audio = inputs["video_audio"]
        self.assertEqual((audio["sample_rate"], audio["channels"]), (16000, 1))
        self.assertFalse(audio["deliver_to_user"])
        self.assertEqual(self.source.read_bytes(), b"original video")
        self.assertEqual(progress.emit.call_count, 2)

    def test_scopes_only_prepare_requested_components(self):
        for scope, names in [
            ("images_only", {"video_first_frame"}),
            ("images_and_audio", {"video_first_frame", "video_audio"}),
        ]:
            with self.subTest(scope=scope), mock.patch.object(
                video_inputs.subprocess, "run", side_effect=self.fake_ffmpeg,
            ):
                self.assertEqual(set(self.prepare(scope)), names)
        for scope in ("", "none", "auto"):
            with self.subTest(scope=scope), mock.patch.object(video_inputs.subprocess, "run") as run:
                existing = {"images": []}
                self.assertIs(self.prepare(scope, existing), existing)
                run.assert_not_called()

    def test_failed_extraction_preserves_video_without_stt_fallback(self):
        for failure in (FileNotFoundError("ffmpeg"), subprocess.CompletedProcess([], 1, "", "no audio")):
            with self.subTest(failure=failure):
                kwargs = {"side_effect": failure} if isinstance(failure, Exception) else {"return_value": failure}
                with mock.patch.object(video_inputs.subprocess, "run", **kwargs):
                    inputs = self.prepare("audio_only")
                bundle = self.skill._content_bundle(
                    self.artifacts, processing_inputs=inputs, text_conversion_scope="audio_only",
                )
                self.assertEqual(inputs["video_audio"]["error_code"], "video_audio_extraction_failed")
                self.assertFalse(inputs["video_audio"]["retryable"])
                self.assertEqual(bundle["delivery_policy"], "best_effort_components")
                self.assertEqual(len(bundle["conversion_failures"]), 1)
                self.assertNotIn("followup_policy", bundle)
                self.assertTrue(self.source.is_file())
                self.assertEqual(list(self.root.glob("*.wav")), [])

    def test_missing_prepared_audio_never_sends_original_video_to_stt(self):
        bundle = self.skill._content_bundle(
            self.artifacts, processing_inputs=None, text_conversion_scope="audio_only",
        )
        self.assertNotIn("followup_policy", bundle)

    def test_invalid_audio_is_not_declared_available(self):
        def malformed(command, **kwargs):
            Path(command[-1]).write_bytes(b"not wav")
            return subprocess.CompletedProcess(command, 0, "", "")
        with mock.patch.object(video_inputs.subprocess, "run", side_effect=malformed):
            inputs = self.prepare("audio_only")
        self.assertEqual(inputs["video_audio"]["status"], "unavailable")
        self.assertEqual(list(self.root.glob("*.wav")), [])

    def test_repeated_extraction_does_not_overwrite_previous_input(self):
        with mock.patch.object(video_inputs.subprocess, "run", side_effect=self.fake_ffmpeg):
            first = self.prepare("audio_only")["video_audio"]["path"]
            second = self.prepare("audio_only")["video_audio"]["path"]
        self.assertNotEqual(first, second)
        self.assertTrue(Path(first).is_file())

    def test_extract_only_does_not_require_a_local_asr_engine(self):
        request = {"context": {"workspace_root": str(self.root)}}
        with mock.patch.object(self.skill, "_available_transcription_engines", return_value=()):
            command = self.skill._build_transcribe_command(request, {
                "input_path": str(self.source), "extract_audio_only": True,
            }, self.root)
        self.assertIn("--extract-only", command)

    @unittest.skipUnless(shutil.which("ffmpeg"), "FFmpeg is not installed")
    def test_real_ffmpeg_extracts_valid_audio_and_handles_silent_video(self):
        audio_video = self.root / "fixture.mov"
        for with_audio in (True, False):
            command = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
                       "-f", "lavfi", "-i", "color=size=32x32:rate=10:duration=0.2"]
            if with_audio:
                command += ["-f", "lavfi", "-i", "sine=frequency=440:duration=0.2",
                            "-c:a", "pcm_s16le"]
            command += ["-c:v", "mpeg4", str(audio_video)]
            subprocess.run(command, check=True, capture_output=True)
            self.artifacts[0]["path"] = str(audio_video)
            inputs = self.prepare("audio_only")
            if with_audio:
                with wave.open(inputs["video_audio"]["path"], "rb") as audio:
                    self.assertGreater(audio.getnframes(), 0)
                    self.assertEqual((audio.getnchannels(), audio.getframerate()), (1, 16000))
            else:
                self.assertEqual(inputs["video_audio"]["status"], "unavailable")
            self.assertTrue(audio_video.is_file())


if __name__ == "__main__":
    unittest.main()
