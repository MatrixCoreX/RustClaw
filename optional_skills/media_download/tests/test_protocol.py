import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SKILL_ROOT = Path(__file__).parents[1]
ENTRYPOINT = SKILL_ROOT / "src" / "main.py"


def load_skill_module():
    spec = importlib.util.spec_from_file_location("media_download_skill_main", ENTRYPOINT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AdapterTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.skill = load_skill_module()

    def test_capabilities_use_structured_contract(self) -> None:
        response = self.skill.respond(
            {
                "request_id": "capabilities-1",
                "args": {"action": "capabilities"},
                "context": None,
                "user_id": 1,
                "chat_id": 1,
            }
        )

        self.assertEqual(response["request_id"], "capabilities-1")
        self.assertEqual(response["status"], "ok")
        self.assertIsNone(response["error_text"])
        self.assertEqual(response["extra"]["schema_version"], 1)
        self.assertEqual(response["extra"]["source_skill"], "media_download")
        self.assertEqual(response["extra"]["status"], "ok")
        self.assertFalse(response["extra"]["system_browser_cookies"])
        self.assertIn("download", response["extra"]["actions"])

    def test_download_command_disables_system_browser_cookies(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            command = self.skill._build_download_command(
                {
                    "share": "https://example.test/public-post",
                    "platform": "auto",
                },
                Path(directory),
                resolve_only=False,
            )

        self.assertIn("--no-system-browser-cookies", command)
        self.assertIn("--no-simplify-chinese", command)
        self.assertIn("--no-ocr-images", command)
        self.assertNotIn("--transcribe", command)
        self.assertNotIn("--extract-audio", command)
        self.assertNotIn("--cookies", command)
        self.assertEqual(command[-1], "https://example.test/public-post")

    def test_download_command_never_adds_text_or_audio_postprocessing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            command = self.skill._build_download_command(
                {
                    "share": "https://example.test/public-post",
                    "ocr_images": True,
                    "transcribe": True,
                    "extract_audio": True,
                    "translate": True,
                },
                Path(directory),
                resolve_only=False,
            )

        self.assertIn("--no-ocr-images", command)
        self.assertNotIn("--transcribe", command)
        self.assertNotIn("--extract-audio", command)
        self.assertNotIn("--whisper-translate", command)

    def test_download_command_preserves_complete_share_text(self) -> None:
        share_text = "复制这条消息，打开快手看看 https://v.kuaishou.com/example/ 更多内容"
        with tempfile.TemporaryDirectory() as directory:
            command = self.skill._build_download_command(
                {"share": share_text, "platform": "auto"},
                Path(directory),
                resolve_only=False,
            )

        self.assertEqual(command[-1], share_text)

    def test_failed_download_preserves_a_readable_reason(self) -> None:
        failure = self.skill._failure_from_process(
            "download",
            1,
            "no downloadable media was exposed by the public page",
            [],
            not_applied=True,
        )

        self.assertEqual(failure.error_code, "media_not_found")
        self.assertIn("no downloadable media", str(failure))
        self.assertEqual(
            failure.details["diagnostics"],
            "no downloadable media was exposed by the public page",
        )
        self.assertEqual(failure.details["failure_phase"], "execution_no_effect")
        self.assertFalse(failure.details["side_effect_applied"])

    def test_failed_tool_rolls_back_partial_output_and_proves_no_effect(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            artifacts = workspace / "artifacts"
            workspace.mkdir()

            def fake_run(command, **kwargs):
                output_dir = Path(command[command.index("--output-dir") + 1])
                output_dir.mkdir(parents=True, exist_ok=True)
                (output_dir / "partial.mp4").write_bytes(b"partial")
                return subprocess.CompletedProcess(
                    command,
                    1,
                    "",
                    "no downloadable media was exposed by the public page",
                )

            request = {
                "request_id": "download-failure-rollback",
                "args": {
                    "action": "download",
                    "share": "https://example.test/missing",
                },
                "context": {
                    "artifact_output_directory": str(artifacts),
                    "workspace_root": str(workspace),
                    "permissions": {"allow_path_outside_workspace": False},
                },
                "user_id": 1,
                "chat_id": 1,
            }
            with (
                mock.patch.object(self.skill.subprocess, "run", side_effect=fake_run),
                self.assertRaises(self.skill.SkillFailure) as raised,
            ):
                self.skill.respond(request)

            self.assertEqual(list(artifacts.rglob("*")), [])

        failure = raised.exception
        self.assertEqual(failure.error_code, "media_not_found")
        self.assertEqual(failure.details["failure_phase"], "execution_no_effect")
        self.assertFalse(failure.details["side_effect_applied"])
        self.assertEqual(failure.details["artifacts"], [])

    def test_local_input_outside_workspace_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            workspace.mkdir()
            outside = root / "outside.mp4"
            outside.write_bytes(b"not-media")
            request = {
                "context": {
                    "workspace_root": str(workspace),
                    "permissions": {"allow_path_outside_workspace": False},
                }
            }

            with self.assertRaises(self.skill.SkillFailure) as raised:
                self.skill._input_path(request, str(outside))

        self.assertEqual(raised.exception.error_code, "permission_denied")

    def test_ocr_uses_distinct_default_name_and_source_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            artifacts = workspace / "artifacts"
            workspace.mkdir()
            image = workspace / "page.jpg"
            image.write_bytes(b"image")

            def fake_run(command, **kwargs):
                output = Path(command[command.index("--output") + 1])
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_text("识别结果\n", encoding="utf-8")
                return subprocess.CompletedProcess(command, 0, "", "")

            request = {
                "request_id": "ocr-1",
                "args": {
                    "action": "ocr",
                    "input_paths": [str(image)],
                },
                "context": {
                    "artifact_output_directory": str(artifacts),
                    "workspace_root": str(workspace),
                    "permissions": {"allow_path_outside_workspace": False},
                },
                "user_id": 1,
                "chat_id": 1,
            }
            with mock.patch.object(self.skill.subprocess, "run", side_effect=fake_run) as runner:
                response = self.skill.respond(request)

        self.assertEqual(response["status"], "ok")
        self.assertEqual(response["extra"]["artifacts"][0]["filename"], "image_text_ocr.txt")
        self.assertEqual(
            response["extra"]["artifacts"][0]["recognition_source"],
            "local_ocr",
        )
        self.assertEqual(
            response["extra"]["recognition"],
            {"source": "local_ocr", "engine": "tesseract"},
        )
        self.assertEqual(
            response["extra"]["delivery"],
            {"intent": "artifact", "deliver_to_user": True},
        )
        command = runner.call_args.args[0]
        self.assertEqual(
            Path(command[command.index("--output") + 1]).name,
            "image_text_ocr.txt",
        )

    def test_download_returns_new_files_as_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            artifacts = workspace / "artifacts"
            workspace.mkdir()

            def fake_run(command, **kwargs):
                output_dir = Path(command[command.index("--output-dir") + 1])
                output_dir.mkdir(parents=True, exist_ok=True)
                (output_dir / "public-video.mp4").write_bytes(b"video")
                return subprocess.CompletedProcess(command, 0, "", "")

            request = {
                "request_id": "download-1",
                "args": {
                    "action": "download",
                    "share": "https://example.test/public-post",
                },
                "context": {
                    "artifact_output_directory": str(artifacts),
                    "workspace_root": str(workspace),
                    "permissions": {"allow_path_outside_workspace": False},
                },
                "user_id": 1,
                "chat_id": 1,
            }
            with mock.patch.object(self.skill.subprocess, "run", side_effect=fake_run) as runner:
                response = self.skill.respond(request)

        self.assertEqual(response["status"], "ok")
        self.assertEqual(response["extra"]["count"], 1)
        self.assertEqual(response["extra"]["artifacts"][0]["filename"], "public-video.mp4")
        self.assertEqual(response["extra"]["artifacts"][0]["mime_type"], "video/mp4")
        self.assertEqual(
            response["extra"]["delivery"],
            {"intent": "artifact", "deliver_to_user": True},
        )
        command = runner.call_args.args[0]
        self.assertIn("--no-system-browser-cookies", command)
        self.assertNotIn("shell", runner.call_args.kwargs)

    def test_download_can_save_without_returning_delivery_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            artifacts = workspace / "artifacts"
            workspace.mkdir()

            def fake_run(command, **kwargs):
                output_dir = Path(command[command.index("--output-dir") + 1])
                output_dir.mkdir(parents=True, exist_ok=True)
                (output_dir / "saved-only.mp4").write_bytes(b"video")
                return subprocess.CompletedProcess(command, 0, "", "")

            request = {
                "request_id": "download-save-only-1",
                "args": {
                    "action": "download",
                    "share": "https://example.test/public-post 不要发我",
                    "deliver_to_user": False,
                },
                "context": {
                    "artifact_output_directory": str(artifacts),
                    "workspace_root": str(workspace),
                    "permissions": {"allow_path_outside_workspace": False},
                },
                "user_id": 1,
                "chat_id": 1,
            }
            with mock.patch.object(self.skill.subprocess, "run", side_effect=fake_run):
                response = self.skill.respond(request)

        self.assertEqual(response["status"], "ok")
        self.assertEqual(response["extra"]["artifacts"], [])
        self.assertEqual(response["extra"]["saved_files"][0]["filename"], "saved-only.mp4")
        self.assertEqual(
            response["extra"]["delivery"],
            {"intent": "save_only", "deliver_to_user": False},
        )
        self.assertIn("Saved locally at:", response["text"])
        self.assertIn("saved-only.mp4", response["text"])


class JsonlProtocolTest(unittest.TestCase):
    def run_protocol(self, request: dict) -> tuple[subprocess.CompletedProcess[str], dict]:
        completed = subprocess.run(
            [sys.executable, str(ENTRYPOINT)],
            input=json.dumps(request, ensure_ascii=False) + "\n",
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        lines = completed.stdout.splitlines()
        action = request.get("args", {}).get("action")
        progress_actions = {
            "capabilities",
            "download",
            "resolve",
            "transcribe",
            "ocr",
            "prepare_x",
        }
        expected_lines = 2 if action in progress_actions else 1
        self.assertEqual(len(lines), expected_lines, completed.stdout)
        if expected_lines == 2:
            progress = json.loads(lines[0])
            self.assertEqual(progress["record_type"], "skill_progress")
            self.assertEqual(progress["request_id"], request["request_id"])
            self.assertEqual(progress["detail_key"], "media_download.precheck.starting")
        return completed, json.loads(lines[-1])

    def test_emits_progress_before_exactly_one_final_json_line(self) -> None:
        completed, response = self.run_protocol(
            {
                "request_id": "protocol-1",
                "args": {"action": "capabilities"},
                "context": None,
                "user_id": 1,
                "chat_id": 1,
            }
        )

        self.assertEqual(completed.returncode, 0)
        self.assertEqual(completed.stderr, "")
        self.assertEqual(response["request_id"], "protocol-1")
        self.assertEqual(response["status"], "ok")

    def test_error_uses_canonical_fields(self) -> None:
        _, response = self.run_protocol(
            {
                "request_id": "protocol-error-1",
                "args": {"action": "unknown"},
                "context": None,
                "user_id": 1,
                "chat_id": 1,
            }
        )

        self.assertEqual(response["status"], "error")
        self.assertTrue(response["error_text"])
        self.assertEqual(
            set(
                key
                for key in response["extra"]
                if key
                in {
                    "schema_version",
                    "source_skill",
                    "status",
                    "error_code",
                    "message_key",
                    "retryable",
                }
            ),
            {
                "schema_version",
                "source_skill",
                "status",
                "error_code",
                "message_key",
                "retryable",
            },
        )
        self.assertEqual(response["extra"]["error_code"], "unsupported_action")
        self.assertEqual(response["extra"]["failure_phase"], "pre_dispatch")
        self.assertFalse(response["extra"]["side_effect_applied"])
        self.assertNotIn("error_kind", response["extra"])
        self.assertNotIn("code", response["extra"])


if __name__ == "__main__":
    unittest.main()
