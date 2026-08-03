import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from contextlib import redirect_stderr
from unittest import mock


TOOL_ROOT = Path(__file__).parents[1] / "src" / "tool"
MODULE_PATH = TOOL_ROOT / "video_transcriber.py"


def load_video_transcriber_module():
    sys.path.insert(0, str(TOOL_ROOT))
    spec = importlib.util.spec_from_file_location("media_download_video_transcriber", MODULE_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ManagedFunAsrModelTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.transcriber = load_video_transcriber_module()

    @staticmethod
    def prepare_model(cache: Path, *relative: str) -> Path:
        model = cache.joinpath(*relative)
        model.mkdir(parents=True)
        (model / "model.pt").write_bytes(b"model")
        (model / "config.yaml").write_text("model: test\n", encoding="utf-8")
        return model

    def test_managed_model_aliases_resolve_to_private_cache(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            sensevoice = self.prepare_model(cache, "iic", "SenseVoiceSmall")
            vad = self.prepare_model(
                cache, "iic", "speech_fsmn_vad_zh-cn-16k-common-pytorch"
            )
            with mock.patch.dict(os.environ, {"MODELSCOPE_CACHE": str(cache)}):
                self.assertEqual(
                    self.transcriber.resolve_managed_funasr_model("iic/SenseVoiceSmall"),
                    str(sensevoice),
                )
                self.assertEqual(
                    self.transcriber.resolve_managed_funasr_model("fsmn-vad"),
                    str(vad),
                )

    def test_parent_progress_marker_is_machine_readable(self) -> None:
        output = io.StringIO()
        with redirect_stderr(output):
            self.transcriber.emit_parent_progress(
                "media_download.transcribe.extracting_audio",
                1,
                3,
            )

        line = output.getvalue().strip()
        self.assertTrue(line.startswith(self.transcriber.PARENT_PROGRESS_PREFIX))
        payload = json.loads(line[len(self.transcriber.PARENT_PROGRESS_PREFIX) :])
        self.assertEqual(payload["detail_key"], "media_download.transcribe.extracting_audio")
        self.assertEqual((payload["current"], payload["total"]), (1, 3))

    def test_incomplete_managed_model_fails_without_runtime_download(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.dict(os.environ, {"MODELSCOPE_CACHE": directory}):
                with self.assertRaisesRegex(
                    self.transcriber.VideoTranscribeError, "Repair or reinstall"
                ):
                    self.transcriber.resolve_managed_funasr_model("iic/SenseVoiceSmall")

    def test_funasr_receives_local_models_and_disables_update_checks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / "cache"
            sensevoice = self.prepare_model(cache, "iic", "SenseVoiceSmall")
            vad = self.prepare_model(
                cache, "iic", "speech_fsmn_vad_zh-cn-16k-common-pytorch"
            )
            audio = root / "sample.wav"
            transcript = root / "sample.txt"
            audio.write_bytes(b"audio")
            captured: dict[str, object] = {}

            class FakeAutoModel:
                def __init__(self, **kwargs):
                    captured.update(kwargs)

                def generate(self, **_kwargs):
                    return [{"text": "本地模型可用"}]

            fake_funasr = types.ModuleType("funasr")
            fake_funasr.AutoModel = FakeAutoModel
            with mock.patch.dict(os.environ, {"MODELSCOPE_CACHE": str(cache)}), mock.patch.dict(
                sys.modules, {"funasr": fake_funasr}
            ):
                self.transcriber.transcribe_audio_with_funasr(
                    audio,
                    transcript,
                    simplify_chinese=False,
                )

            self.assertEqual(captured["model"], str(sensevoice))
            self.assertEqual(captured["vad_model"], str(vad))
            self.assertTrue(captured["disable_update"])
            self.assertFalse(captured["check_latest"])
            self.assertEqual(captured["vad_kwargs"], {"check_latest": False})
            self.assertEqual(transcript.read_text(encoding="utf-8"), "本地模型可用\n")


if __name__ == "__main__":
    unittest.main()
