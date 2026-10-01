import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from contextlib import redirect_stdout
from unittest import mock


SKILL_ROOT = Path(__file__).parents[1]
MODULE_PATH = SKILL_ROOT / "src" / "main.py"


def load_module():
    spec = importlib.util.spec_from_file_location("local_asr_main", MODULE_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class LocalAsrProtocolTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = load_module()

    def test_capabilities_does_not_import_heavy_backend(self):
        with mock.patch.dict(sys.modules, {"funasr": None, "torch": None}):
            response = self.module.respond(
                {"request_id": "cap-1", "args": {"action": "capabilities"}}
            )
        self.assertEqual(response["status"], "ok")
        self.assertEqual(response["extra"]["engines"], ["whisper", "funasr"])

    def test_capabilities_process_emits_exactly_one_final_json_record(self):
        request = json.dumps(
            {
                "request_id": "cap-process-1",
                "args": {"action": "capabilities"},
                "context": None,
                "user_id": 1,
                "chat_id": 1,
            }
        )

        completed = subprocess.run(
            [sys.executable, str(MODULE_PATH)],
            input=request + "\n",
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        records = [json.loads(line) for line in completed.stdout.splitlines()]
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["request_id"], "cap-process-1")
        self.assertEqual(records[0]["status"], "ok")
        self.assertEqual(records[0]["extra"]["source_skill"], "local_asr")

    def test_missing_input_process_uses_canonical_error_fields(self):
        request = json.dumps(
            {
                "request_id": "missing-process-1",
                "args": {
                    "action": "transcribe",
                    "input_path": "missing.wav",
                },
                "context": {"workspace_root": "/tmp"},
                "user_id": 1,
                "chat_id": 1,
            }
        )

        completed = subprocess.run(
            [sys.executable, str(MODULE_PATH)],
            input=request + "\n",
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        records = [json.loads(line) for line in completed.stdout.splitlines()]
        self.assertEqual(len(records), 1)
        response = records[0]
        self.assertEqual(response["status"], "error")
        self.assertEqual(response["extra"]["error_code"], "not_found")
        self.assertEqual(response["extra"]["message_key"], "local_asr.error.input_not_found")
        self.assertFalse(response["extra"]["retryable"])
        self.assertNotIn("error_kind", response["extra"])
        self.assertNotIn("code", response["extra"])

    def test_funasr_uses_private_models_without_update_checks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            storage = root / "storage"
            for relative in (
                ("iic", "SenseVoiceSmall"),
                ("iic", "speech_fsmn_vad_zh-cn-16k-common-pytorch"),
            ):
                model = storage.joinpath("modelscope", *relative)
                model.mkdir(parents=True)
                (model / "model.pt").write_bytes(b"model")
                (model / "config.yaml").write_text("model: test\n", encoding="utf-8")
            audio = root / "audio.wav"
            audio.write_bytes(b"wav")
            output = root / "transcript.txt"
            captured = {}

            class FakeAutoModel:
                def __init__(self, **kwargs):
                    captured.update(kwargs)

                def generate(self, **_kwargs):
                    return [{"text": "本地模型可用"}]

            fake = types.ModuleType("funasr")
            fake.AutoModel = FakeAutoModel
            with mock.patch.dict(sys.modules, {"funasr": fake}):
                self.module._transcribe_funasr(audio, output, storage, {})

            self.assertTrue(captured["disable_update"])
            self.assertFalse(captured["check_latest"])
            self.assertEqual(captured["vad_kwargs"], {"check_latest": False})
            self.assertEqual(output.read_text(encoding="utf-8"), "本地模型可用\n")

    def test_transcribe_returns_review_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.wav"
            source.write_bytes(b"wav")
            artifacts = root / "artifacts"
            storage = root / "storage"
            storage.mkdir()

            def fake_transcribe(_audio, output, _args):
                output.write_text("raw transcript\n", encoding="utf-8")

            request = {
                "request_id": "asr-1",
                "args": {
                    "action": "transcribe",
                    "input_path": str(source),
                    "engine": "whisper",
                    "response_language": "en",
                },
                "context": {
                    "workspace_root": str(root),
                    "artifact_output_directory": str(artifacts),
                    "skill_storage": {
                        "storage_kind": "directory",
                        "directory_path": str(storage),
                    },
                },
            }
            with mock.patch.object(self.module, "_transcribe_whisper", fake_transcribe):
                output = io.StringIO()
                with redirect_stdout(output):
                    response = self.module.respond(
                        request, self.module.ProgressReporter("asr-1")
                    )

            frames = [json.loads(line) for line in output.getvalue().splitlines()]
            self.assertEqual(len(frames), 3)
            self.assertEqual([frame["sequence"] for frame in frames], [1, 2, 3])
            self.assertEqual(
                [frame["detail_key"] for frame in frames],
                [
                    "local_asr.extracting_audio",
                    "local_asr.recognizing_speech",
                    "local_asr.completed",
                ],
            )
            self.assertTrue(all(frame["record_type"] == "skill_progress" for frame in frames))
            self.assertEqual(response["status"], "ok")
            self.assertEqual(
                response["extra"]["transcription_review"]["raw_text"],
                "raw transcript",
            )
            self.assertEqual(response["extra"]["artifacts"], [])
            self.assertEqual(
                response["extra"]["saved_files"][-1]["artifact_role"],
                "transcript_text",
            )
            self.assertEqual(
                response["extra"]["transcription_review"]["delivery"]["text_filename"],
                "source_transcript.txt",
            )

    def test_long_multilingual_transcript_is_not_truncated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "long.wav"
            source.write_bytes(b"wav")
            artifacts = root / "artifacts"
            storage = root / "storage"
            storage.mkdir()
            raw_text = ("第一段 English العربية 123.45\n" * 800).strip()

            def fake_transcribe(_audio, output, _args):
                output.write_text(raw_text + "\n", encoding="utf-8")

            request = {
                "request_id": "asr-long-1",
                "args": {
                    "action": "transcribe",
                    "input_path": str(source),
                    "engine": "whisper",
                    "response_language": "preserve-source-language",
                },
                "context": {
                    "workspace_root": str(root),
                    "artifact_output_directory": str(artifacts),
                    "skill_storage": {
                        "storage_kind": "directory",
                        "directory_path": str(storage),
                    },
                },
            }

            with mock.patch.object(self.module, "_transcribe_whisper", fake_transcribe):
                response = self.module.respond(request)

            review = response["extra"]["transcription_review"]
            self.assertEqual(review["raw_text"], raw_text)
            self.assertEqual(review["raw_character_count"], len(raw_text))
            transcript_path = Path(response["extra"]["saved_files"][-1]["path"])
            self.assertEqual(transcript_path.read_text(encoding="utf-8").strip(), raw_text)

    def test_long_input_segments_merge_in_source_order_and_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "long.wav"
            source.write_bytes(b"wav")
            artifacts = root / "artifacts"
            storage = root / "storage"
            storage.mkdir()

            def fake_split(_source, temporary):
                paths = [temporary / "segment_00000.wav", temporary / "segment_00001.wav"]
                for path in paths:
                    path.write_bytes(b"segment")
                return paths

            def fake_transcribe(audio, output, _args):
                output.write_text(
                    "第一段" if audio.name.endswith("00000.wav") else "Second segment",
                    encoding="utf-8",
                )

            request = {
                "request_id": "asr-segmented-1",
                "args": {
                    "action": "transcribe",
                    "input_path": str(source),
                    "engine": "whisper",
                    "response_language": "preserve-source-language",
                },
                "context": {
                    "workspace_root": str(root),
                    "artifact_output_directory": str(artifacts),
                    "skill_storage": {
                        "storage_kind": "directory",
                        "directory_path": str(storage),
                    },
                },
            }

            with mock.patch.object(
                self.module, "_probe_duration_seconds", return_value=1200.0
            ), mock.patch.object(
                self.module, "_split_audio", side_effect=fake_split
            ), mock.patch.object(
                self.module, "_transcribe_whisper", side_effect=fake_transcribe
            ):
                response = self.module.respond(request)

            self.assertEqual(
                response["extra"]["transcription_review"]["raw_text"],
                "第一段\nSecond segment",
            )
            self.assertEqual(
                response["extra"]["chunking"],
                {
                    "segmented": True,
                    "segment_seconds": 480,
                    "segment_count": 2,
                    "source_duration_seconds": 1200.0,
                    "merge_order": "source_order",
                },
            )
            self.assertEqual(list(artifacts.glob(".local-asr-*")), [])


if __name__ == "__main__":
    unittest.main()
