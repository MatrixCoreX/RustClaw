import importlib.util
import io
import json
from contextlib import redirect_stdout
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
        self.assertEqual(
            response["extra"]["image_article_posts"]["default_outputs"],
            ["original_images", "article_text"],
        )
        self.assertTrue(response["extra"]["image_article_posts"]["ocr_is_separate"])
        self.assertEqual(
            response["extra"]["image_article_posts"]["inline_text_max_characters_exclusive"],
            200,
        )
        self.assertTrue(response["extra"]["transcription_engines"]["whisper"]["default"])

    def test_progress_reporter_emits_ordered_machine_frames(self) -> None:
        output = io.StringIO()
        reporter = self.skill.ProgressReporter("progress-1")
        with redirect_stdout(output):
            reporter.emit(
                "media_download.transcribe.extracting_audio",
                current=1,
                total=3,
            )
            reporter.emit(
                "media_download.transcribe.recognizing_speech",
                current=2,
                total=3,
            )

        frames = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual([frame["sequence"] for frame in frames], [1, 2])
        self.assertEqual(frames[0]["record_type"], "skill_progress")
        self.assertEqual(frames[0]["params"]["step_id"], "extract_audio")
        self.assertEqual(frames[0]["params"]["step_status"], "in_progress")
        self.assertEqual(frames[1]["detail_key"], "media_download.transcribe.recognizing_speech")
        self.assertEqual(frames[1]["params"]["step_id"], "transcribe_speech")
        self.assertEqual(frames[1]["params"]["step_status"], "in_progress")
        self.assertEqual((frames[1]["current"], frames[1]["total"]), (2, 3))

    def test_child_progress_is_forwarded_without_user_prose(self) -> None:
        forwarded: list[tuple[str, int, int]] = []
        line = (
            self.skill.CHILD_PROGRESS_PREFIX
            + '{"detail_key":"media_download.transcribe.recognizing_speech","current":2,"total":3}'
        )

        consumed = self.skill._child_progress(
            line,
            lambda key, current, total: forwarded.append((key, current, total)),
        )

        self.assertTrue(consumed)
        self.assertEqual(
            forwarded,
            [("media_download.transcribe.recognizing_speech", 2, 3)],
        )

    def test_intel_macos_capabilities_keep_whisper_and_disable_funasr_package(self) -> None:
        with mock.patch.object(self.skill.platform, "system", return_value="Darwin"), mock.patch.object(
            self.skill.platform, "machine", return_value="x86_64"
        ):
            extra = self.skill._capabilities_extra()

        self.assertTrue(extra["transcription_engines"]["whisper"]["supported"])
        self.assertFalse(extra["transcription_engines"]["funasr"]["supported"])
        self.assertEqual(
            extra["transcription_engines"]["funasr"]["unavailable_reason_code"],
            "platform_binary_unavailable",
        )
        self.assertEqual(extra["available_transcription_engines"], ["whisper"])
        self.assertEqual(extra["installed_dependencies"]["transcription_alternative"], [])

    def test_intel_macos_rejects_unavailable_funasr_before_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_path = root / "sample.mp4"
            input_path.write_bytes(b"video")
            request = {
                "context": {
                    "workspace_root": str(root),
                    "permissions": {"allow_path_outside_workspace": False},
                }
            }
            with mock.patch.object(self.skill.platform, "system", return_value="Darwin"), mock.patch.object(
                self.skill.platform, "machine", return_value="x86_64"
            ), self.assertRaises(self.skill.SkillFailure) as raised:
                self.skill._build_transcribe_command(
                    request,
                    {"input_path": "sample.mp4", "engine": "funasr"},
                    root / "artifacts",
                )

        failure = raised.exception
        self.assertEqual(failure.error_code, "dependency_unavailable")
        self.assertEqual(failure.details["requested_engine"], "funasr")
        self.assertEqual(failure.details["available_engines"], ["whisper"])
        self.assertEqual(
            failure.details["unavailable_reason_code"],
            "platform_binary_unavailable",
        )

    def test_intel_macos_requirement_roots_exclude_the_funasr_stack(self) -> None:
        requirements = (SKILL_ROOT / "requirements.in").read_text(encoding="utf-8")
        marker = 'sys_platform != "darwin" or platform_machine != "x86_64"'

        for package in ("funasr", "modelscope", "torch", "torchaudio"):
            line = next(
                candidate
                for candidate in requirements.splitlines()
                if candidate.startswith(f"{package}==")
            )
            self.assertIn(marker, line)

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

    def test_private_directory_storage_routes_modelscope_cache(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "artifacts"
            storage = root / "private-storage"
            artifacts.mkdir()
            storage.mkdir()
            request = {
                "context": {
                    "skill_storage": {
                        "storage_kind": "directory",
                        "directory_path": str(storage),
                    }
                }
            }
            self.assertEqual(self.skill._skill_storage_directory(request), storage)
            completed = subprocess.CompletedProcess(["tool"], 0, stdout="ok", stderr="")
            with mock.patch.object(self.skill.subprocess, "run", return_value=completed) as run:
                self.skill._run_tool("transcribe", ["tool"], artifacts, 30, storage)
            self.assertEqual(
                run.call_args.kwargs["env"]["MODELSCOPE_CACHE"],
                str(storage / "modelscope"),
            )

    def test_download_routes_profile_checkpoints_to_private_skill_storage(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            storage = root / "private-storage"
            storage.mkdir()
            command = self.skill._build_download_command(
                {"share": "https://example.test/public-profile"},
                root / "artifacts",
                resolve_only=False,
                storage_directory=storage,
            )
            resolve_command = self.skill._build_download_command(
                {"share": "https://example.test/public-profile"},
                root / "artifacts",
                resolve_only=True,
                storage_directory=storage,
            )

        checkpoint_index = command.index("--profile-checkpoint-dir")
        self.assertEqual(
            command[checkpoint_index + 1],
            str(storage / "profile_checkpoints"),
        )
        self.assertNotIn("--profile-checkpoint-dir", resolve_command)

    def test_partial_profile_checkpoint_is_reported_as_a_persisted_side_effect(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "artifacts"
            storage = root / "private-storage"
            artifacts.mkdir()
            storage.mkdir()

            def failed_profile_run(*_args, **_kwargs):
                pointer = (
                    storage
                    / "profile_checkpoints"
                    / "douyin"
                    / "profile-id"
                    / "current.json"
                )
                pointer.parent.mkdir(parents=True)
                pointer.write_text(
                    json.dumps(
                        {
                            "state": "partial",
                            "sequence": 3,
                            "sha256": "a" * 64,
                        }
                    ),
                    encoding="utf-8",
                )
                return subprocess.CompletedProcess(
                    ["tool"],
                    1,
                    stdout="",
                    stderr="profile collection failed",
                )

            with mock.patch.object(
                self.skill.subprocess,
                "run",
                side_effect=failed_profile_run,
            ):
                with self.assertRaises(self.skill.SkillFailure) as raised:
                    self.skill._run_tool(
                        "download",
                        ["tool"],
                        artifacts,
                        None,
                        storage,
                    )

        details = raised.exception.details
        self.assertEqual(details["failure_phase"], "execution_partial")
        self.assertTrue(details["side_effect_applied"])
        self.assertEqual(details["profile_collection"]["state"], "partial")
        self.assertTrue(
            details["profile_collection"]["resumable_checkpoint_preserved"]
        )

    def test_media_operation_has_no_internal_deadline_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "artifacts"
            request = {
                "request_id": "download-no-internal-deadline",
                "args": {
                    "action": "download",
                    "share": "https://example.test/public-post",
                },
                "context": {
                    "artifact_output_directory": str(artifacts),
                    "workspace_root": str(root),
                    "permissions": {"allow_path_outside_workspace": False},
                },
                "user_id": 1,
                "chat_id": 1,
            }
            completed = subprocess.CompletedProcess(["tool"], 0, stdout="ok", stderr="")
            with mock.patch.object(self.skill.subprocess, "run", return_value=completed) as run:
                self.skill.respond(request)

        self.assertIsNone(run.call_args.kwargs["timeout"])

    def test_download_ignores_an_explicit_operation_deadline(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "artifacts"
            request = {
                "request_id": "download-explicit-deadline",
                "args": {
                    "action": "download",
                    "share": "https://example.test/public-post",
                    "operation_timeout_seconds": 3_000,
                },
                "context": {
                    "artifact_output_directory": str(artifacts),
                    "workspace_root": str(root),
                    "permissions": {"allow_path_outside_workspace": False},
                },
                "user_id": 1,
                "chat_id": 1,
            }
            completed = subprocess.CompletedProcess(["tool"], 0, stdout="ok", stderr="")
            with mock.patch.object(self.skill.subprocess, "run", return_value=completed) as run:
                self.skill.respond(request)

        self.assertIsNone(run.call_args.kwargs["timeout"])

    def test_non_download_operation_honors_an_explicit_deadline(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "artifacts"
            request = {
                "request_id": "resolve-explicit-deadline",
                "args": {
                    "action": "resolve",
                    "share": "https://example.test/public-post",
                    "operation_timeout_seconds": 3_000,
                },
                "context": {
                    "artifact_output_directory": str(artifacts),
                    "workspace_root": str(root),
                    "permissions": {"allow_path_outside_workspace": False},
                },
                "user_id": 1,
                "chat_id": 1,
            }
            completed = subprocess.CompletedProcess(
                ["tool"],
                0,
                stdout="https://example.test/media.mp4\n",
                stderr="",
            )
            with mock.patch.object(self.skill.subprocess, "run", return_value=completed) as run:
                self.skill.respond(request)

        self.assertEqual(run.call_args.kwargs["timeout"], 3_000)

    def test_media_operation_accepts_an_explicit_deadline_beyond_one_hour(self) -> None:
        self.assertEqual(
            self.skill._optional_integer(
                {"operation_timeout_seconds": 7_200},
                "operation_timeout_seconds",
                minimum=5,
                maximum=2_592_000,
            ),
            7_200,
        )

    def test_media_operation_accepts_max_deadline_without_platform_overflow(self) -> None:
        completed = self.skill._run_process(
            [sys.executable, "-c", "print('ok')"],
            dict(self.skill.os.environ),
            2_592_000,
        )

        self.assertEqual(completed.returncode, 0)
        self.assertEqual(completed.stdout.strip(), "ok")

    def test_media_operation_rejects_an_unreasonable_explicit_deadline(self) -> None:
        with self.assertRaisesRegex(self.skill.SkillFailure, "between 5 and 2592000"):
            self.skill._optional_integer(
                {"operation_timeout_seconds": 2_592_001},
                "operation_timeout_seconds",
                minimum=5,
                maximum=2_592_000,
            )

    def test_transcribe_command_defaults_to_local_whisper(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_path = root / "sample.mp4"
            input_path.write_bytes(b"video")
            request = {
                "context": {
                    "workspace_root": str(root),
                    "permissions": {"allow_path_outside_workspace": False},
                }
            }
            command = self.skill._build_transcribe_command(
                request,
                {"input_path": "sample.mp4"},
                root / "artifacts",
            )

        self.assertEqual(command[command.index("--engine") + 1], "whisper")
        self.assertNotIn("--extract-only", command)
        self.assertEqual(command[-1], str(input_path))

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
            output_rollback_ok=True,
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
                output.write_text("识别结果" * 50 + "\n", encoding="utf-8")
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

    def test_ocr_rejects_video_input_before_process_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory)
            video = workspace / "downloaded.mp4"
            video.write_bytes(b"video")
            request = {
                "context": {
                    "workspace_root": str(workspace),
                    "permissions": {"allow_path_outside_workspace": False},
                }
            }

            with self.assertRaises(self.skill.SkillFailure) as raised:
                self.skill._build_ocr_command(
                    request,
                    {"input_paths": [str(video)]},
                    workspace / "artifacts",
                )

        self.assertEqual(raised.exception.error_code, "invalid_input_media_type")
        self.assertEqual(
            raised.exception.message_key,
            "media_download.error.ocr_requires_image",
        )

    def test_short_ocr_result_is_delivered_inline(self) -> None:
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
                output.write_text("短OCR结果", encoding="utf-8")
                return subprocess.CompletedProcess(command, 0, "", "")

            request = {
                "request_id": "ocr-inline-1",
                "args": {"action": "ocr", "input_paths": [str(image)]},
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

            self.assertFalse((artifacts / "image_text_ocr.txt").exists())

        self.assertEqual(response["status"], "ok")
        self.assertEqual(response["extra"]["artifacts"], [])
        self.assertIn("短OCR结果", response["text"])
        self.assertEqual(
            response["extra"]["delivery"],
            {"intent": "model_synthesis", "deliver_to_user": True},
        )
        self.assertEqual(
            response["extra"]["recognition_delivery"],
            {
                "mode": "inline",
                "source": "local_ocr",
                "engine": "tesseract",
                "character_count": 6,
                "text": "短OCR结果",
            },
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
        self.assertEqual(
            response["extra"]["content_bundle"]["followup_policy"],
            {
                "text_conversion_action": "transcribe_audio",
                "capability": "media_download.transcribe",
                "input_field": "input_path",
                "never_use_image_ocr": True,
            },
        )
        command = runner.call_args.args[0]
        self.assertIn("--no-system-browser-cookies", command)
        self.assertNotIn("shell", runner.call_args.kwargs)

    def test_download_classifies_image_article_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            artifacts = workspace / "artifacts"
            workspace.mkdir()

            def fake_run(command, **kwargs):
                output_dir = Path(command[command.index("--output-dir") + 1])
                output_dir.mkdir(parents=True, exist_ok=True)
                (output_dir / "note_01.jpg").write_bytes(b"one")
                (output_dir / "note_02.jpg").write_bytes(b"two")
                (output_dir / "note_article.txt").write_text(
                    "平台：小红书\n\n正文：\n" + "长" * 200,
                    encoding="utf-8",
                )
                return subprocess.CompletedProcess(command, 0, "", "")

            request = {
                "request_id": "download-image-article-1",
                "args": {
                    "action": "download",
                    "share": "https://www.xiaohongshu.com/explore/example",
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
        self.assertEqual(
            response["extra"]["content_bundle"],
            {
                "schema_version": 1,
                "kind": "image_article",
                "image_count": 2,
                "video_count": 0,
                "article_count": 1,
                "other_file_count": 0,
                "inline_article_count": 0,
            },
        )
        artifacts_by_role = {
            artifact["artifact_role"]: artifact
            for artifact in response["extra"]["artifacts"]
            if "artifact_role" in artifact
        }
        self.assertEqual(artifacts_by_role["article_text"]["mime_type"], "text/plain")
        self.assertEqual(
            artifacts_by_role["article_text"]["content_source"],
            "platform_post",
        )
        self.assertIn("original_image", artifacts_by_role)

    def test_short_platform_article_is_delivered_inline_only_for_this_skill(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            artifacts = workspace / "artifacts"
            workspace.mkdir()

            def fake_run(command, **kwargs):
                output_dir = Path(command[command.index("--output-dir") + 1])
                output_dir.mkdir(parents=True, exist_ok=True)
                (output_dir / "short_note.jpg").write_bytes(b"image")
                (output_dir / "short_note_article.txt").write_text(
                    "平台：小红书\n作者：测试作者\n\n正文：\n这是一段少于二百字的平台原始正文。\n",
                    encoding="utf-8",
                )
                return subprocess.CompletedProcess(command, 0, "", "")

            request = {
                "request_id": "download-short-article-1",
                "args": {
                    "action": "download",
                    "share": "https://www.xiaohongshu.com/explore/example",
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

            self.assertFalse((artifacts / "short_note_article.txt").exists())

        self.assertEqual(response["status"], "ok")
        self.assertEqual([item["artifact_role"] for item in response["extra"]["artifacts"]], ["original_image"])
        self.assertIn("这是一段少于二百字的平台原始正文。", response["text"])
        self.assertEqual(
            response["extra"]["article_delivery"],
            {
                "mode": "inline",
                "content_source": "platform_post",
                "character_count": 17,
                "text": "这是一段少于二百字的平台原始正文。",
            },
        )
        self.assertEqual(response["extra"]["content_bundle"]["kind"], "image_article")
        self.assertEqual(response["extra"]["content_bundle"]["article_count"], 1)
        self.assertEqual(response["extra"]["content_bundle"]["inline_article_count"], 1)

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
            self.assertEqual(progress["params"]["step_id"], "media_precheck")
            self.assertEqual(progress["params"]["step_status"], "in_progress")
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
