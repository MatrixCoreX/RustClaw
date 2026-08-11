from __future__ import annotations

import json
import mimetypes
import os
from pathlib import Path
import platform
import queue
import re
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import zipfile
from typing import Any, Callable
from urllib.parse import urlsplit


SKILL_NAME = "media_download"
SCHEMA_VERSION = 1
TOOL_DIR = Path(__file__).resolve().parent / "tool"
PRIVATE_BIN_DIR = Path(sys.executable).resolve().parent
os.environ["PATH"] = os.pathsep.join(
    [str(PRIVATE_BIN_DIR), os.environ.get("PATH", "")]
).rstrip(os.pathsep)
SUPPORTED_ACTIONS = (
    "capabilities",
    "download",
    "resolve",
    "transcribe",
    "ocr",
    "prepare_x",
)
SUPPORTED_PLATFORMS = ("auto", "douyin", "kuaishou", "xiaohongshu", "tiktok", "youtube")
MAX_DIAGNOSTIC_CHARS = 4_000
INLINE_TEXT_MAX_CHARS = 200
IMAGE_ARCHIVE_THRESHOLD = 9
SUBPROCESS_TIMEOUT_SLICE_SECONDS = 24 * 60 * 60
CHILD_PROGRESS_PREFIX = "__MEDIA_DOWNLOAD_PROGRESS__:"
PROGRESS_STEP_IDS = {
    "precheck": "media_precheck",
    "download": "download_media",
    "resolve": "resolve_media",
    "extract_audio": "extract_audio",
    "transcribe": "transcribe_speech",
    "ocr": "recognize_images",
    "prepare_x": "prepare_media",
}
OCR_IMAGE_EXTENSIONS = {
    ".avif",
    ".bmp",
    ".gif",
    ".jpeg",
    ".jpg",
    ".png",
    ".tif",
    ".tiff",
    ".webp",
}
IMAGE_TEXT_REVISION_PROMPT = (
    Path("prompts") / "layers" / "overlays" / "image_text_revision_prompt.md"
)
IMAGE_TEXT_REVISION_CHUNK_CHARS = 6_000


class SkillFailure(Exception):
    def __init__(
        self,
        message: str,
        *,
        error_code: str,
        message_key: str,
        retryable: bool = False,
        details: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(message)
        self.error_code = error_code
        self.message_key = message_key
        self.retryable = retryable
        self.details = details or {}


class ProgressReporter:
    def __init__(self, request_id: str) -> None:
        self.request_id = request_id
        self.sequence = 0

    def emit(
        self,
        detail_key: str,
        *,
        params: dict[str, Any] | None = None,
        current: int | None = None,
        total: int | None = None,
    ) -> None:
        self.sequence += 1
        progress_params = _progress_metadata(detail_key)
        if params:
            progress_params.update(params)
        frame: dict[str, Any] = {
            "schema_version": 1,
            "record_type": "skill_progress",
            "request_id": self.request_id,
            "sequence": self.sequence,
            "kind": "progress",
            "detail_key": detail_key,
            "params": progress_params,
        }
        if current is not None and total is not None:
            frame["current"] = current
            frame["total"] = total
        sys.stdout.write(
            json.dumps(frame, ensure_ascii=False, separators=(",", ":")) + "\n"
        )
        sys.stdout.flush()

    def forward_child(self, detail_key: str, current: int, total: int) -> None:
        self.emit(detail_key, current=current, total=total)


def _progress_metadata(detail_key: str) -> dict[str, str]:
    step_name = ""
    if detail_key == "media_download.precheck.starting":
        step_name = "precheck"
    elif detail_key == "media_download.transcribe.extracting_audio":
        step_name = "extract_audio"
    elif detail_key == "media_download.transcribe.recognizing_speech":
        step_name = "transcribe"
    else:
        parts = detail_key.split(".")
        if len(parts) == 3 and parts[0] == "media_download":
            step_name = parts[1]
    step_id = PROGRESS_STEP_IDS.get(step_name)
    if step_id is None:
        return {}
    return {
        "step_id": step_id,
        "step_status": "completed" if detail_key.endswith(".completed") else "in_progress",
    }


def _mark_not_applied(failure: SkillFailure, failure_phase: str) -> SkillFailure:
    failure.details.setdefault("failure_phase", failure_phase)
    failure.details.setdefault("side_effect_applied", False)
    return failure


def _args(request: dict[str, Any]) -> dict[str, Any]:
    args = request.get("args")
    if not isinstance(args, dict):
        raise SkillFailure(
            "args must be an object",
            error_code="invalid_args",
            message_key="media_download.error.invalid_args",
        )
    return args


def _string(
    args: dict[str, Any],
    name: str,
    *,
    required: bool = False,
    default: str | None = None,
    max_length: int = 20_000,
) -> str | None:
    value = args.get(name, default)
    if value is None:
        if required:
            raise SkillFailure(
                f"missing required argument: {name}",
                error_code="missing_argument",
                message_key=f"media_download.error.missing_{name}",
            )
        return None
    if not isinstance(value, str) or not value.strip():
        raise SkillFailure(
            f"{name} must be a non-empty string",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    value = value.strip()
    if len(value) > max_length or "\x00" in value:
        raise SkillFailure(
            f"{name} is too long or contains invalid characters",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    return value


def _bool(args: dict[str, Any], name: str, default: bool = False) -> bool:
    value = args.get(name, default)
    if not isinstance(value, bool):
        raise SkillFailure(
            f"{name} must be a boolean",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    return value


def _integer(
    args: dict[str, Any],
    name: str,
    *,
    default: int,
    minimum: int,
    maximum: int,
) -> int:
    value = args.get(name, default)
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise SkillFailure(
            f"{name} must be an integer between {minimum} and {maximum}",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    return value


def _optional_integer(
    args: dict[str, Any],
    name: str,
    *,
    minimum: int,
    maximum: int,
) -> int | None:
    if name not in args:
        return None
    value = args[name]
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise SkillFailure(
            f"{name} must be an integer between {minimum} and {maximum}",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    return value


def _number(
    args: dict[str, Any],
    name: str,
    *,
    default: float,
    minimum: float,
    maximum: float,
) -> float:
    value = args.get(name, default)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise SkillFailure(
            f"{name} must be a number",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    value = float(value)
    if not minimum <= value <= maximum:
        raise SkillFailure(
            f"{name} must be between {minimum:g} and {maximum:g}",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    return value


def _choice(args: dict[str, Any], name: str, choices: tuple[str, ...], default: str) -> str:
    value = args.get(name, default)
    if not isinstance(value, str) or value not in choices:
        raise SkillFailure(
            f"{name} must be one of: {', '.join(choices)}",
            error_code="invalid_args",
            message_key=f"media_download.error.invalid_{name}",
        )
    return value


def _artifact_output_directory(request: dict[str, Any]) -> Path:
    context = request.get("context")
    if not isinstance(context, dict):
        raise SkillFailure(
            "runtime artifact output directory is unavailable",
            error_code="invalid_args",
            message_key="media_download.error.artifact_directory_unavailable",
        )
    raw = context.get("artifact_output_directory")
    if not isinstance(raw, str) or not raw.strip():
        raise SkillFailure(
            "runtime artifact output directory is unavailable",
            error_code="invalid_args",
            message_key="media_download.error.artifact_directory_unavailable",
        )
    path = Path(raw).expanduser().resolve()
    path.mkdir(parents=True, exist_ok=True)
    return path


def _skill_storage_directory(request: dict[str, Any]) -> Path | None:
    context = request.get("context")
    if not isinstance(context, dict):
        return None
    storage = context.get("skill_storage")
    if not isinstance(storage, dict) or storage.get("storage_kind") != "directory":
        return None
    raw = storage.get("directory_path")
    if not isinstance(raw, str) or not raw.strip():
        return None
    path = Path(raw).expanduser()
    return path if path.is_absolute() else None


def _input_path(request: dict[str, Any], raw: str) -> Path:
    context = request.get("context")
    workspace_root = None
    allow_outside = False
    if isinstance(context, dict):
        workspace = context.get("workspace_root")
        if isinstance(workspace, str) and workspace.strip():
            workspace_root = Path(workspace).expanduser().resolve()
        permissions = context.get("permissions")
        if isinstance(permissions, dict):
            allow_outside = permissions.get("allow_path_outside_workspace") is True

    path = Path(raw).expanduser()
    if not path.is_absolute() and workspace_root is not None:
        path = workspace_root / path
    path = path.resolve()
    if not path.exists():
        raise SkillFailure(
            f"input path does not exist: {path}",
            error_code="not_found",
            message_key="media_download.error.input_not_found",
        )
    if workspace_root is not None and not allow_outside:
        try:
            path.relative_to(workspace_root)
        except ValueError as error:
            raise SkillFailure(
                f"input path is outside the allowed workspace: {path}",
                error_code="permission_denied",
                message_key="media_download.error.path_outside_workspace",
            ) from error
    return path


def _safe_output_name(args: dict[str, Any]) -> str | None:
    value = _string(args, "output_name", max_length=255)
    if value is None:
        return None
    if Path(value).name != value or value in {".", ".."}:
        raise SkillFailure(
            "output_name must be a plain filename without a directory",
            error_code="invalid_args",
            message_key="media_download.error.invalid_output_name",
        )
    return value


def _safe_text_output_name(args: dict[str, Any], default: str) -> str:
    value = _safe_output_name(args) or default
    if Path(value).suffix.lower() != ".txt":
        raise SkillFailure(
            "output_name for image text recognition must end with .txt",
            error_code="invalid_args",
            message_key="media_download.error.invalid_output_name",
        )
    return value


def _tool(script: str) -> list[str]:
    entrypoint = TOOL_DIR / script
    if not entrypoint.is_file():
        raise SkillFailure(
            f"bundled tool entrypoint is missing: {script}",
            error_code="dependency_unavailable",
            message_key="media_download.error.tool_missing",
        )
    return [sys.executable, str(entrypoint)]


def _build_download_command(
    args: dict[str, Any],
    output_dir: Path,
    *,
    resolve_only: bool,
    storage_directory: Path | None = None,
) -> list[str]:
    share = _string(args, "share", required=True)
    assert share is not None
    platform = _choice(args, "platform", SUPPORTED_PLATFORMS, "auto")
    network_timeout = _number(
        args,
        "network_timeout_seconds",
        default=20.0,
        minimum=1.0,
        maximum=120.0,
    )
    profile_interval = _number(
        args,
        "profile_interval_seconds",
        default=5.0,
        minimum=0.0,
        maximum=60.0,
    )
    profile_limit = args.get("profile_limit", 20)
    if profile_limit != "all" and (
        isinstance(profile_limit, bool)
        or not isinstance(profile_limit, int)
        or not 1 <= profile_limit <= 500
    ):
        raise SkillFailure(
            "profile_limit must be an integer from 1 to 500, or 'all'",
            error_code="invalid_args",
            message_key="media_download.error.invalid_profile_limit",
        )

    command = _tool("media_downloader.py")
    command.extend(
        [
            "--output-dir",
            str(output_dir),
            "--platform",
            platform,
            "--timeout",
            f"{network_timeout:g}",
            "--profile-limit",
            str(profile_limit),
            "--profile-interval",
            f"{profile_interval:g}",
            "--no-system-browser-cookies",
            "--no-simplify-chinese",
        ]
    )
    if not _bool(args, "browser_fallback", True):
        command.append("--no-browser-fallback")

    if not resolve_only and storage_directory is not None:
        command.extend(
            [
                "--profile-checkpoint-dir",
                str(storage_directory / "profile_checkpoints"),
            ]
        )

    if resolve_only:
        command.extend(["--print-url", "--no-ocr-images"])
    else:
        output_name = _safe_output_name(args)
        if output_name:
            command.extend(["--output-name", output_name])
        flag_map = {
            "save_meta": "--save-meta",
            "show_info": "--show-info",
            "x_compatible": "--x-compatible",
            "overwrite": "--overwrite",
        }
        for name, flag in flag_map.items():
            if _bool(args, name, False):
                command.append(flag)
        command.append("--no-ocr-images")
    command.append(share)
    return command


def _build_transcribe_command(request: dict[str, Any], args: dict[str, Any], output_dir: Path) -> list[str]:
    engine = _choice(args, "engine", ("whisper", "funasr"), "whisper")
    available_engines = _available_transcription_engines()
    if engine not in available_engines:
        raise SkillFailure(
            f"transcription engine is unavailable on this platform: {engine}",
            error_code="dependency_unavailable",
            message_key="media_download.error.dependency_unavailable",
            details={
                "requested_engine": engine,
                "available_engines": list(available_engines),
                "unavailable_reason_code": "platform_binary_unavailable",
            },
        )
    raw = _string(args, "input_path", required=True, max_length=4_096)
    assert raw is not None
    input_path = _input_path(request, raw)
    language = _string(args, "language", default="auto", max_length=32) or "auto"
    command = _tool("video_transcriber.py")
    command.extend(
        [
            "--output-dir",
            str(output_dir),
            "--engine",
            engine,
            "--language",
            language,
            "--no-progress",
            "--no-simplify-chinese",
        ]
    )
    flag_map = {
        "extract_audio_only": "--extract-only",
        "translate": "--translate",
        "fast": "--fast",
        "no_gpu": "--no-gpu",
        "timestamps": "--timestamps",
        "overwrite": "--overwrite",
    }
    for name, flag in flag_map.items():
        if _bool(args, name, False):
            command.append(flag)
    command.append(str(input_path))
    return command


def _build_ocr_command(request: dict[str, Any], args: dict[str, Any], output_dir: Path) -> list[str]:
    raw_paths = args.get("input_paths")
    if not isinstance(raw_paths, list) or not raw_paths:
        raise SkillFailure(
            "input_paths must be a non-empty array of image paths",
            error_code="invalid_args",
            message_key="media_download.error.invalid_input_paths",
        )
    paths = []
    for raw in raw_paths:
        if not isinstance(raw, str) or not raw.strip() or len(raw) > 4_096:
            raise SkillFailure(
                "each input_paths item must be a non-empty path string",
                error_code="invalid_args",
                message_key="media_download.error.invalid_input_paths",
            )
        path = _input_path(request, raw.strip())
        if path.suffix.lower() not in OCR_IMAGE_EXTENSIONS:
            raise SkillFailure(
                "ocr accepts image files only; video and audio files must remain media outputs",
                error_code="invalid_input_media_type",
                message_key="media_download.error.ocr_requires_image",
                details={"input_extension": path.suffix.lower()},
            )
        paths.append(path)

    language = _string(args, "language", default="auto", max_length=64) or "auto"
    psm = _integer(args, "psm", default=6, minimum=0, maximum=13)
    min_confidence = _number(
        args,
        "min_line_confidence",
        default=30.0,
        minimum=-1.0,
        maximum=100.0,
    )
    command = _tool("image_ocr.py")
    command.extend(
        [
            "--output",
            str(output_dir / _safe_text_output_name(args, "image_text_ocr.txt")),
            "--output-dir",
            str(output_dir),
            "--language",
            language,
            "--psm",
            str(psm),
            "--min-line-confidence",
            f"{min_confidence:g}",
        ]
    )
    if not _bool(args, "preprocess", True):
        command.append("--no-preprocess")
    if _bool(args, "overwrite", False):
        command.append("--overwrite")
    command.extend(str(path) for path in paths)
    return command


def _build_prepare_x_command(request: dict[str, Any], args: dict[str, Any], output_dir: Path) -> list[str]:
    raw = _string(args, "input_path", required=True, max_length=4_096)
    assert raw is not None
    input_path = _input_path(request, raw)
    crf = _integer(args, "crf", default=23, minimum=16, maximum=35)
    command = _tool("x_transcoder.py")
    command.extend(["--output-dir", str(output_dir), "--crf", str(crf)])
    flag_map = {
        "check_only": "--check",
        "force": "--force",
        "overwrite": "--overwrite",
    }
    for name, flag in flag_map.items():
        if _bool(args, name, False):
            command.append(flag)
    command.append(str(input_path))
    return command


def _snapshot(directory: Path) -> dict[Path, tuple[int, int]]:
    snapshot: dict[Path, tuple[int, int]] = {}
    for path in directory.rglob("*"):
        try:
            stat = path.stat()
        except OSError:
            continue
        if path.is_file():
            snapshot[path.resolve()] = (stat.st_size, stat.st_mtime_ns)
    return snapshot


def _artifact(path: Path) -> dict[str, Any]:
    mime_type, _ = mimetypes.guess_type(path.name)
    artifact = {
        "path": str(path),
        "filename": path.name,
        "mime_type": mime_type or "application/octet-stream",
        "size_bytes": path.stat().st_size,
    }
    if path.name == "profile_downloads.json":
        artifact["artifact_role"] = "profile_manifest"
    elif path.suffix.lower() == ".txt" and re.search(r"_article(?:\.\d+)?$", path.stem):
        artifact.update(
            {
                "artifact_role": "article_text",
                "content_source": "platform_post",
            }
        )
    elif artifact["mime_type"].startswith("image/"):
        artifact["artifact_role"] = "original_image"
    elif artifact["mime_type"].startswith("video/"):
        artifact["artifact_role"] = "original_video"
    elif path.suffix.lower() == ".txt" and path.stem.endswith("_transcript"):
        artifact["artifact_role"] = "transcript_text"
    elif artifact["mime_type"].startswith("audio/"):
        artifact["artifact_role"] = "extracted_audio"
    elif path.suffix.lower() == ".zip":
        artifact["artifact_role"] = "archive"
    elif path.suffix.lower() == ".json":
        artifact["artifact_role"] = "metadata"
    return artifact


def _available_archive_path(output_dir: Path) -> Path:
    candidate = output_dir / "image_bundle.zip"
    if not candidate.exists():
        return candidate
    for index in range(2, 1_000):
        candidate = output_dir / f"image_bundle_{index}.zip"
        if not candidate.exists():
            return candidate
    raise SkillFailure(
        "cannot allocate an image archive filename",
        error_code="artifact_packaging_failed",
        message_key="media_download.error.artifact_packaging_failed",
        details={"failure_phase": "execution_partial", "side_effect_applied": True},
    )


def _package_large_image_delivery(
    artifacts: list[dict[str, Any]],
    output_dir: Path,
) -> tuple[list[dict[str, Any]], dict[str, Any] | None, dict[str, Any] | None]:
    images = [item for item in artifacts if item.get("artifact_role") == "original_image"]
    if not images:
        return artifacts, None, None

    if len(images) <= IMAGE_ARCHIVE_THRESHOLD:
        return artifacts, None, None

    delivery = {
        "mode": "archive",
        "threshold": IMAGE_ARCHIVE_THRESHOLD,
        "image_count": len(images),
    }

    article_files = [
        item for item in artifacts if item.get("artifact_role") == "article_text"
    ]
    archive_path = _available_archive_path(output_dir)
    try:
        with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_STORED) as archive:
            for item in [*images, *article_files]:
                path = Path(str(item["path"]))
                archive.write(path, arcname=path.name)
    except (OSError, KeyError, TypeError, zipfile.BadZipFile) as error:
        try:
            archive_path.unlink(missing_ok=True)
        except OSError:
            pass
        raise SkillFailure(
            f"image archive creation failed: {error}",
            error_code="artifact_packaging_failed",
            message_key="media_download.error.artifact_packaging_failed",
            details={"failure_phase": "execution_partial", "side_effect_applied": True},
        ) from error

    archive_artifact = _artifact(archive_path)
    archive_artifact.update(
        {
            "artifact_role": "image_archive",
            "contained_image_count": len(images),
            "contained_article_count": len(article_files),
        }
    )
    delivery.update(
        {
            "archive_filename": archive_artifact["filename"],
            "archive_path": archive_artifact["path"],
            "article_included": bool(article_files),
        }
    )
    processing_inputs = {
        "images": [
            {
                "path": item["path"],
                "filename": item["filename"],
                "mime_type": item["mime_type"],
                "size_bytes": item["size_bytes"],
            }
            for item in images
        ],
        "image_count": len(images),
        "ordered": True,
    }
    return (
        [archive_artifact]
        + [item for item in artifacts if item.get("artifact_role") != "original_image"],
        delivery,
        processing_inputs,
    )


def _changed_artifact_paths(
    before: dict[Path, tuple[int, int]],
    after: dict[Path, tuple[int, int]],
) -> list[Path]:
    changed = [path for path, signature in after.items() if before.get(path) != signature]
    return sorted(
        changed,
        key=lambda path: (path.name != "profile_downloads.json", str(path)),
    )


def _profile_collection_summary(
    artifacts: list[dict[str, Any]],
) -> dict[str, Any] | None:
    manifest = next(
        (item for item in artifacts if item.get("artifact_role") == "profile_manifest"),
        None,
    )
    if manifest is None:
        return None
    try:
        payload = json.loads(Path(str(manifest["path"])).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError, KeyError, TypeError):
        return None
    if not isinstance(payload, dict):
        return None
    collection = payload.get("collection")
    if not isinstance(collection, dict):
        return None
    return {
        "schema_version": 1,
        "state": payload.get("state"),
        "platform": payload.get("platform"),
        "item_count": payload.get("item_count"),
        "completed_count": payload.get("completed_count"),
        "failed_count": payload.get("failed_count"),
        "cursor": collection.get("cursor"),
        "checkpoint_sequence": payload.get("checkpoint_sequence"),
        "checkpoint_digest": payload.get("checkpoint_digest"),
    }


def _content_bundle(
    artifacts: list[dict[str, Any]],
    inline_article: dict[str, Any] | None = None,
    image_delivery: dict[str, Any] | None = None,
) -> dict[str, Any]:
    counts = {
        "image_count": 0,
        "video_count": 0,
        "article_count": 0,
        "other_file_count": 0,
    }
    roles = {
        "original_image": "image_count",
        "original_video": "video_count",
        "article_text": "article_count",
    }
    for artifact in artifacts:
        role = str(artifact.get("artifact_role") or "")
        if role == "image_archive":
            contained = artifact.get("contained_image_count")
            if isinstance(contained, int) and not isinstance(contained, bool) and contained > 0:
                counts["image_count"] += contained
            continue
        count_key = roles.get(role, "other_file_count")
        counts[count_key] += 1
    inline_article_count = 1 if inline_article is not None else 0
    counts["article_count"] += inline_article_count
    if counts["image_count"] and counts["article_count"]:
        kind = "image_article"
    elif counts["image_count"]:
        kind = "images"
    elif counts["video_count"]:
        kind = "video"
    else:
        kind = "files"
    bundle = {
        "schema_version": 1,
        "kind": kind,
        **counts,
        "inline_article_count": inline_article_count,
    }
    if image_delivery is not None:
        bundle["image_delivery"] = image_delivery
    if kind == "video":
        original_video = next(
            (
                artifact
                for artifact in artifacts
                if artifact.get("artifact_role") == "original_video"
            ),
            None,
        )
        bundle["followup_policy"] = {
            "text_conversion_action": "transcribe_audio",
            "capability": "media_download.transcribe",
            "input_field": "input_path",
            **(
                {"input_value": original_video["path"]}
                if original_video is not None and original_video.get("path")
                else {}
            ),
            "never_use_image_ocr": True,
        }
    return bundle


def _consume_short_text_artifact(
    artifacts: list[dict[str, Any]],
    artifact: dict[str, Any] | None,
    *,
    body_marker: str | None = None,
) -> tuple[list[dict[str, Any]], str | None]:
    if artifact is None:
        return artifacts, None
    path = Path(str(artifact["path"]))
    try:
        document = path.read_text(encoding="utf-8")
    except OSError:
        return artifacts, None
    body = (
        document.partition(body_marker)[2].strip()
        if body_marker and body_marker in document
        else document.strip()
    )
    if not body or len(body) >= INLINE_TEXT_MAX_CHARS:
        return artifacts, None
    try:
        path.unlink()
    except OSError:
        return artifacts, None
    return (
        [item for item in artifacts if item is not artifact],
        body,
    )


def _inline_short_article(
    artifacts: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    article = next(
        (item for item in artifacts if item.get("artifact_role") == "article_text"),
        None,
    )
    remaining, body = _consume_short_text_artifact(
        artifacts,
        article,
        body_marker="\n正文：\n",
    )
    if body is None:
        return artifacts, None
    return (
        remaining,
        {
            "mode": "inline",
            "content_source": "platform_post",
            "character_count": len(body),
            "text": body,
        },
    )


def _inline_short_ocr(
    artifacts: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    ocr_artifact = next(
        (item for item in artifacts if item.get("recognition_source") == "local_ocr"),
        None,
    )
    remaining, text = _consume_short_text_artifact(artifacts, ocr_artifact)
    if text is None:
        return artifacts, None
    return (
        remaining,
        {
            "mode": "inline",
            "source": "local_ocr",
            "engine": "tesseract",
            "character_count": len(text),
            "text": text,
        },
    )


def _image_text_revision_prompt() -> str | None:
    workspace = os.environ.get("WORKSPACE_ROOT", "").strip()
    if not workspace:
        return None
    try:
        prompt = (Path(workspace) / IMAGE_TEXT_REVISION_PROMPT).read_text(encoding="utf-8")
    except OSError:
        return None
    return prompt if prompt.strip() else None


def _split_revision_chunks(text: str) -> list[str]:
    text = text.strip()
    if not text:
        return []
    chunks: list[str] = []
    start = 0
    while start < len(text):
        end = min(start + IMAGE_TEXT_REVISION_CHUNK_CHARS, len(text))
        if end < len(text):
            floor = start + IMAGE_TEXT_REVISION_CHUNK_CHARS // 2
            boundary = next(
                (index for index in range(end - 1, floor - 1, -1) if text[index].isspace()),
                None,
            )
            if boundary is not None:
                end = boundary + 1
        chunk = text[start:end]
        if chunk.strip():
            chunks.append(chunk)
        start = end
    return chunks


def _revision_numeric_tokens(text: str) -> list[str]:
    tokens: list[str] = []
    current: list[str] = []
    for character in text:
        if character.isnumeric():
            current.append(character)
        elif current:
            tokens.append("".join(current))
            current = []
    if current:
        tokens.append("".join(current))
    return tokens


def _revision_preserves_source(raw_text: str, reviewed_text: str) -> bool:
    if not reviewed_text.strip():
        return False
    if _revision_numeric_tokens(raw_text) != _revision_numeric_tokens(reviewed_text):
        return False
    raw_count = sum(1 for character in raw_text if not character.isspace())
    reviewed_count = sum(1 for character in reviewed_text if not character.isspace())
    if raw_count < 40:
        return reviewed_count <= raw_count * 2 + 16
    return int(raw_count * 0.6) <= reviewed_count <= int(raw_count * 1.5 + 0.999)


def _join_reviewed_chunks(source_chunks: list[str], reviewed_chunks: list[str]) -> str:
    assembled: list[str] = []
    for index, reviewed in enumerate(reviewed_chunks):
        assembled.append(reviewed.strip())
        if index < len(source_chunks) - 1:
            boundary = re.search(r"\s+$", source_chunks[index])
            if boundary:
                assembled.append(boundary.group(0))
    return "".join(assembled).strip()


def _internal_llm_revision(prompt: str) -> tuple[str | None, dict[str, Any]]:
    url = os.environ.get("AGENT_INTERNAL_LLM_URL", "").strip()
    token = os.environ.get("AGENT_INTERNAL_LLM_TOKEN", "").strip()
    if not url or not token:
        return None, {"status": "unavailable", "reviewed_by_model": False}
    try:
        configured_timeout = int(os.environ.get("SKILL_TIMEOUT_SECONDS", "60") or "60")
    except ValueError:
        configured_timeout = 60
    timeout = min(max(configured_timeout, 5), 120)
    body = json.dumps(
        {
            "skill_name": SKILL_NAME,
            "prompt_source": "skills/media_download/image_text_revision",
            "prompt": prompt,
            "temperature": 0.0,
            "max_tokens": 8192,
        },
        ensure_ascii=False,
    ).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={
            "content-type": "application/json",
            "x-agent-internal-llm-token": token,
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except (OSError, ValueError, urllib.error.URLError) as error:
        return None, {
            "status": "fallback_raw",
            "reviewed_by_model": False,
            "error_code": "model_review_failed",
            "diagnostics": _diagnostics(str(error)),
        }
    data = payload.get("data") if isinstance(payload, dict) and payload.get("ok") is True else None
    reviewed = data.get("text") if isinstance(data, dict) else None
    if not isinstance(reviewed, str) or not reviewed.strip():
        return None, {
            "status": "fallback_raw",
            "reviewed_by_model": False,
            "error_code": "model_review_empty",
        }
    return reviewed.strip(), {
        "status": "reviewed",
        "reviewed_by_model": True,
        "provider": data.get("provider"),
        "model": data.get("model"),
    }


def _review_local_ocr_artifact(
    artifacts: list[dict[str, Any]],
) -> dict[str, Any]:
    artifact = next(
        (item for item in artifacts if item.get("recognition_source") == "local_ocr"),
        None,
    )
    if artifact is None:
        return {"status": "skipped_missing_artifact", "reviewed_by_model": False}
    path = Path(str(artifact.get("path") or ""))
    try:
        raw_text = path.read_text(encoding="utf-8").strip()
    except OSError as error:
        return {
            "status": "fallback_raw",
            "reviewed_by_model": False,
            "error_code": "ocr_text_read_failed",
            "diagnostics": _diagnostics(str(error)),
        }
    template = _image_text_revision_prompt()
    chunks = _split_revision_chunks(raw_text)
    if not template or not chunks:
        return {
            "status": "fallback_raw",
            "reviewed_by_model": False,
            "error_code": "revision_prompt_or_text_unavailable",
        }
    reviewed_chunks: list[str] = []
    review_metadata: dict[str, Any] = {}
    for index, chunk in enumerate(chunks):
        prompt = (
            template.replace("__CHUNK_INDEX__", str(index + 1))
            .replace("__CHUNK_COUNT__", str(len(chunks)))
            .replace("__RAW_RECOGNIZED_TEXT__", chunk)
        )
        reviewed, review_metadata = _internal_llm_revision(prompt)
        if reviewed is None:
            return {**review_metadata, "chunk_count": len(chunks)}
        reviewed_chunks.append(reviewed)
    reviewed_text = _join_reviewed_chunks(chunks, reviewed_chunks)
    if not _revision_preserves_source(raw_text, reviewed_text):
        return {
            "status": "fallback_raw",
            "reviewed_by_model": False,
            "error_code": "revision_integrity_failed",
            "chunk_count": len(chunks),
            "raw_character_count": len(raw_text),
            "reviewed_character_count": len(reviewed_text),
        }
    raw_path = path.with_name(f"{path.stem}_raw{path.suffix}")
    try:
        raw_path.write_text(raw_text + "\n", encoding="utf-8")
        path.write_text(reviewed_text + "\n", encoding="utf-8")
        artifact["size_bytes"] = path.stat().st_size
    except OSError as error:
        return {
            "status": "fallback_raw",
            "reviewed_by_model": False,
            "error_code": "reviewed_text_write_failed",
            "diagnostics": _diagnostics(str(error)),
            "chunk_count": len(chunks),
        }
    return {
        **review_metadata,
        "chunk_count": len(chunks),
        "raw_character_count": len(raw_text),
        "reviewed_character_count": len(reviewed_text),
        "source_language_policy": "preserve_source_language",
        "layout_policy": "semantic_reflow",
        "raw_artifact": {
            "path": str(raw_path),
            "filename": raw_path.name,
            "mime_type": "text/plain",
            "size_bytes": raw_path.stat().st_size,
            "deliver_to_user": False,
        },
    }


def _target_transcript_language(
    request: dict[str, Any],
    args: dict[str, Any],
) -> str:
    explicit = args.get("response_language")
    if isinstance(explicit, str) and explicit.strip():
        return explicit.strip()
    context = request.get("context")
    if isinstance(context, dict):
        for key in ("locale", "language"):
            value = context.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
    memory = args.get("_memory")
    if isinstance(memory, dict):
        value = memory.get("lang_hint")
        if isinstance(value, str) and value.strip():
            return value.strip()
    return "preserve-source-language"


def _prepare_transcription_review_contract(
    request: dict[str, Any],
    args: dict[str, Any],
    artifacts: list[dict[str, Any]],
) -> dict[str, Any] | None:
    if _bool(args, "extract_audio_only", False):
        return None
    transcript_artifact = next(
        (item for item in artifacts if item.get("artifact_role") == "transcript_text"),
        None,
    )
    if transcript_artifact is None:
        raise SkillFailure(
            "transcription completed without a text artifact",
            error_code="transcript_missing",
            message_key="media_download.error.transcript_missing",
            details={"failure_phase": "execution_partial", "side_effect_applied": True},
        )
    transcript_path = Path(str(transcript_artifact["path"]))
    try:
        raw_transcript = transcript_path.read_text(encoding="utf-8").strip()
    except OSError as exc:
        raise SkillFailure(
            f"cannot read local transcript for review: {exc}",
            error_code="transcript_read_failed",
            message_key="media_download.error.transcript_read_failed",
            details={"failure_phase": "execution_partial", "side_effect_applied": True},
        ) from exc
    if not raw_transcript:
        raise SkillFailure(
            "local transcription produced no text",
            error_code="transcript_empty",
            message_key="media_download.error.transcript_empty",
            details={"failure_phase": "execution_partial", "side_effect_applied": True},
        )
    target_language = _target_transcript_language(request, args)
    return {
        "schema_version": 1,
        "required": True,
        "source": "media_download_local_asr",
        "source_engine": _string(args, "engine", max_length=64) or "whisper",
        "raw_text": raw_transcript,
        "raw_character_count": len(raw_transcript),
        "response_language": target_language,
        "corrections": ["recognition_errors", "typos", "broken_sentences"],
        "preserve_meaning": True,
        "delivery": {
            "inline_max_characters_exclusive": INLINE_TEXT_MAX_CHARS,
            "long_text_format": "text/plain; charset=utf-8",
            "long_text_filename": "transcript.txt",
        },
    }


def _diagnostics(stderr: str) -> str:
    value = stderr.strip()
    if len(value) <= MAX_DIAGNOSTIC_CHARS:
        return value
    return value[-MAX_DIAGNOSTIC_CHARS:]


def _rollback_output_changes(
    output_dir: Path,
    before: dict[Path, tuple[int, int]],
    after: dict[Path, tuple[int, int]],
) -> bool:
    changed = [path for path, signature in after.items() if before.get(path) != signature]
    if any(path in before for path in changed):
        return False
    rollback_ok = True
    for path in changed:
        try:
            path.unlink()
        except FileNotFoundError:
            continue
        except OSError:
            rollback_ok = False
    directories = sorted(
        (path for path in output_dir.rglob("*") if path.is_dir()),
        key=lambda path: len(path.parts),
        reverse=True,
    )
    for directory in directories:
        try:
            directory.rmdir()
        except OSError:
            continue
    return rollback_ok and _snapshot(output_dir) == before


def _failure_from_process(
    action: str,
    returncode: int,
    stderr: str,
    artifacts: list[dict[str, Any]],
    *,
    output_rollback_ok: bool = False,
    profile_checkpoint: dict[str, Any] | None = None,
) -> SkillFailure:
    lowered = stderr.lower()
    if any(
        marker in lowered
        for marker in (
            "was not found in path",
            "is required",
            "is not installed",
            "no module named",
        )
    ):
        error_code = "dependency_unavailable"
        message_key = "media_download.error.dependency_unavailable"
    elif "no downloadable" in lowered or "no media" in lowered:
        error_code = "media_not_found"
        message_key = "media_download.error.media_not_found"
    else:
        error_code = "execution_failed"
        message_key = "media_download.error.execution_failed"
    retryable = any(marker in lowered for marker in ("timed out", "timeout", "temporarily", "connection reset"))
    readable = _diagnostics(stderr) or f"{action} failed with exit code {returncode}"
    details: dict[str, Any] = {
        "exit_code": returncode,
        "diagnostics": readable,
        "artifacts": [] if output_rollback_ok else artifacts,
        "failure_phase": (
            "execution_no_effect"
            if output_rollback_ok and profile_checkpoint is None
            else "execution_partial"
        ),
        "side_effect_applied": not output_rollback_ok or profile_checkpoint is not None,
    }
    if profile_checkpoint is not None:
        details["profile_collection"] = profile_checkpoint
    return SkillFailure(
        readable,
        error_code=error_code,
        message_key=message_key,
        retryable=retryable,
        details=details,
    )


def _profile_checkpoint_pointers(
    storage_directory: Path | None,
) -> dict[str, dict[str, Any]]:
    if storage_directory is None:
        return {}
    root = storage_directory / "profile_checkpoints"
    if not root.is_dir():
        return {}
    pointers: dict[str, dict[str, Any]] = {}
    for path in root.glob("*/*/current.json"):
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
            relative = path.relative_to(root).as_posix()
        except (OSError, json.JSONDecodeError, ValueError):
            continue
        if not isinstance(payload, dict):
            continue
        pointers[relative] = {
            "state": payload.get("state"),
            "sequence": payload.get("sequence"),
            "sha256": payload.get("sha256"),
        }
    return pointers


def _changed_profile_checkpoint(
    before: dict[str, dict[str, Any]],
    after: dict[str, dict[str, Any]],
) -> dict[str, Any] | None:
    changed = [value for key, value in after.items() if before.get(key) != value]
    if not changed:
        return None
    latest = max(
        changed,
        key=lambda value: value.get("sequence")
        if isinstance(value.get("sequence"), int)
        else -1,
    )
    return {
        "schema_version": 1,
        "state": latest.get("state"),
        "checkpoint_sequence": latest.get("sequence"),
        "checkpoint_digest": latest.get("sha256"),
        "resumable_checkpoint_preserved": True,
    }


def _run_tool(
    action: str,
    command: list[str],
    output_dir: Path,
    timeout_seconds: int | None,
    storage_directory: Path | None = None,
    progress_callback: Callable[[str, int, int], None] | None = None,
) -> tuple[str, str, list[dict[str, Any]]]:
    before = _snapshot(output_dir)
    checkpoint_before = _profile_checkpoint_pointers(storage_directory)
    environment = os.environ.copy()
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    if storage_directory is not None:
        environment["MODELSCOPE_CACHE"] = str(storage_directory / "modelscope")
    try:
        completed = _run_process(
            command,
            environment,
            timeout_seconds,
            progress_callback=progress_callback,
        )
    except subprocess.TimeoutExpired as error:
        stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else (error.stderr or "")
        after = _snapshot(output_dir)
        changed = _changed_artifact_paths(before, after)
        artifacts = [_artifact(path) for path in changed]
        output_rollback_ok = _rollback_output_changes(output_dir, before, after)
        profile_checkpoint = _changed_profile_checkpoint(
            checkpoint_before,
            _profile_checkpoint_pointers(storage_directory),
        )
        configured_timeout = timeout_seconds if timeout_seconds is not None else error.timeout
        raise SkillFailure(
            f"{action} timed out after {configured_timeout} seconds",
            error_code="timeout",
            message_key="media_download.error.timeout",
            retryable=True,
            details={
                "diagnostics": _diagnostics(stderr),
                "artifacts": [] if output_rollback_ok else artifacts,
                "failure_phase": (
                    "execution_no_effect"
                    if output_rollback_ok and profile_checkpoint is None
                    else "execution_partial"
                ),
                "side_effect_applied": (
                    not output_rollback_ok or profile_checkpoint is not None
                ),
                **(
                    {"profile_collection": profile_checkpoint}
                    if profile_checkpoint is not None
                    else {}
                ),
            },
        ) from error
    after = _snapshot(output_dir)
    changed = _changed_artifact_paths(before, after)
    artifacts = [_artifact(path) for path in changed]
    if completed.returncode != 0:
        output_rollback_ok = _rollback_output_changes(output_dir, before, after)
        profile_checkpoint = _changed_profile_checkpoint(
            checkpoint_before,
            _profile_checkpoint_pointers(storage_directory),
        )
        raise _failure_from_process(
            action,
            completed.returncode,
            completed.stderr,
            artifacts,
            output_rollback_ok=output_rollback_ok,
            profile_checkpoint=profile_checkpoint,
        )
    return completed.stdout, completed.stderr, artifacts


def _run_process(
    command: list[str],
    environment: dict[str, str],
    timeout_seconds: int | None,
    *,
    progress_callback: Callable[[str, int, int], None] | None = None,
) -> subprocess.CompletedProcess[str]:
    if progress_callback is None and (
        timeout_seconds is None or timeout_seconds <= SUBPROCESS_TIMEOUT_SLICE_SECONDS
    ):
        return subprocess.run(
            command,
            cwd=TOOL_DIR,
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
        )

    if progress_callback is not None:
        return _run_process_with_progress(
            command,
            environment,
            timeout_seconds,
            progress_callback,
        )

    process = subprocess.Popen(
        command,
        cwd=TOOL_DIR,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    deadline = time.monotonic() + timeout_seconds
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            process.kill()
            stdout, stderr = process.communicate()
            raise subprocess.TimeoutExpired(
                command,
                timeout_seconds,
                output=stdout,
                stderr=stderr,
            )
        try:
            stdout, stderr = process.communicate(
                timeout=min(remaining, SUBPROCESS_TIMEOUT_SLICE_SECONDS)
            )
            return subprocess.CompletedProcess(
                command,
                process.returncode,
                stdout,
                stderr,
            )
        except subprocess.TimeoutExpired:
            continue


def _child_progress(
    line: str,
    progress_callback: Callable[[str, int, int], None],
) -> bool:
    stripped = line.strip()
    if not stripped.startswith(CHILD_PROGRESS_PREFIX):
        return False
    try:
        payload = json.loads(stripped[len(CHILD_PROGRESS_PREFIX) :])
    except json.JSONDecodeError:
        return False
    if not isinstance(payload, dict):
        return False
    detail_key = payload.get("detail_key")
    current = payload.get("current")
    total = payload.get("total")
    if (
        not isinstance(detail_key, str)
        or not detail_key.startswith("media_download.")
        or isinstance(current, bool)
        or not isinstance(current, int)
        or isinstance(total, bool)
        or not isinstance(total, int)
        or total <= 0
        or current < 0
        or current > total
    ):
        return False
    progress_callback(detail_key, current, total)
    return True


def _run_process_with_progress(
    command: list[str],
    environment: dict[str, str],
    timeout_seconds: int | None,
    progress_callback: Callable[[str, int, int], None],
) -> subprocess.CompletedProcess[str]:
    process = subprocess.Popen(
        command,
        cwd=TOOL_DIR,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert process.stdout is not None
    assert process.stderr is not None
    records: queue.Queue[tuple[str, str | None]] = queue.Queue()

    def drain(name: str, stream: Any) -> None:
        try:
            for line in stream:
                records.put((name, line))
        finally:
            records.put((name, None))

    readers = [
        threading.Thread(
            target=drain,
            args=("stdout", process.stdout),
            daemon=True,
        ),
        threading.Thread(
            target=drain,
            args=("stderr", process.stderr),
            daemon=True,
        ),
    ]
    for reader in readers:
        reader.start()

    stdout_parts: list[str] = []
    stderr_parts: list[str] = []
    closed_streams: set[str] = set()
    deadline = (
        time.monotonic() + timeout_seconds
        if timeout_seconds is not None
        else None
    )
    while len(closed_streams) < 2:
        if deadline is not None and time.monotonic() >= deadline:
            process.kill()
            process.wait()
            for reader in readers:
                reader.join(timeout=1)
            raise subprocess.TimeoutExpired(
                command,
                timeout_seconds,
                output="".join(stdout_parts),
                stderr="".join(stderr_parts),
            )
        wait_seconds = 0.25
        if deadline is not None:
            wait_seconds = max(0.01, min(wait_seconds, deadline - time.monotonic()))
        try:
            stream_name, line = records.get(timeout=wait_seconds)
        except queue.Empty:
            continue
        if line is None:
            closed_streams.add(stream_name)
            continue
        if stream_name == "stderr":
            if not _child_progress(line, progress_callback):
                stderr_parts.append(line)
        else:
            stdout_parts.append(line)

    returncode = process.wait()
    for reader in readers:
        reader.join(timeout=1)
    return subprocess.CompletedProcess(
        command,
        returncode,
        "".join(stdout_parts),
        "".join(stderr_parts),
    )


def _urls(stdout: str) -> list[str]:
    urls: list[str] = []
    for line in stdout.splitlines():
        value = line.strip()
        parsed = urlsplit(value)
        if parsed.scheme in {"http", "https"} and parsed.netloc and value not in urls:
            urls.append(value)
    return urls


def _funasr_prebuilt_supported(
    system_name: str | None = None,
    machine: str | None = None,
) -> bool:
    system = (system_name or platform.system()).strip().lower()
    architecture = (machine or platform.machine()).strip().lower()
    return not (system == "darwin" and architecture in {"x86_64", "amd64"})


def _available_transcription_engines(
    system_name: str | None = None,
    machine: str | None = None,
) -> tuple[str, ...]:
    engines = ["whisper"]
    if _funasr_prebuilt_supported(system_name, machine):
        engines.append("funasr")
    return tuple(engines)


def _capabilities_extra() -> dict[str, Any]:
    available_engines = _available_transcription_engines()
    funasr_supported = "funasr" in available_engines
    return {
        "schema_version": SCHEMA_VERSION,
        "source_skill": SKILL_NAME,
        "status": "ok",
        "action": "capabilities",
        "actions": list(SUPPORTED_ACTIONS),
        "supported_platforms": list(SUPPORTED_PLATFORMS),
        "public_content_only": True,
        "system_browser_cookies": False,
        "image_article_posts": {
            "platforms": ["douyin", "xiaohongshu"],
            "default_outputs": ["original_images", "article_text"],
            "inline_text_max_characters_exclusive": INLINE_TEXT_MAX_CHARS,
            "individual_delivery_max_images": IMAGE_ARCHIVE_THRESHOLD,
            "large_set_delivery": "ordered_zip",
            "article_included_in_large_set_archive": True,
            "ocr_is_separate": True,
        },
        "installed_dependencies": {
            "youtube": ["yt-dlp"],
            "media_processing": ["ffmpeg", "ffprobe"],
            "ocr": ["tesseract", "tesseract_language_data"],
            "browser_fallback": ["chromium_or_chrome"],
            "transcription_alternative": (
                [
                    "funasr",
                    "modelscope",
                    "torch",
                    "modelscope_sensevoice_small",
                    "modelscope_fsmn_vad",
                ]
                if funasr_supported
                else []
            ),
        },
        "host_integrated_dependencies": {
            "default_transcription": ["whisper.cpp"],
        },
        "available_transcription_engines": list(available_engines),
        "transcription_engines": {
            "whisper": {
                "supported": True,
                "default": True,
                "dependency_source": "host",
            },
            "funasr": {
                "supported": funasr_supported,
                "default": False,
                "dependency_source": "skill_package",
                **(
                    {}
                    if funasr_supported
                    else {"unavailable_reason_code": "platform_binary_unavailable"}
                ),
            },
        },
    }


def _success(request_id: str, action: str, text: str, extra: dict[str, Any]) -> dict[str, Any]:
    payload = {
        "schema_version": SCHEMA_VERSION,
        "source_skill": SKILL_NAME,
        "status": "ok",
        "action": action,
    }
    payload.update(extra)
    return {
        "request_id": request_id,
        "status": "ok",
        "text": text,
        "error_text": None,
        "extra": payload,
    }


def _error(request_id: str, failure: SkillFailure) -> dict[str, Any]:
    extra = {
        "schema_version": SCHEMA_VERSION,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": failure.error_code,
        "message_key": failure.message_key,
        "retryable": failure.retryable,
    }
    extra.update(failure.details)
    return {
        "request_id": request_id,
        "status": "error",
        "text": "",
        "error_text": str(failure),
        "extra": extra,
    }


def respond(
    request: dict[str, Any],
    progress: ProgressReporter | None = None,
) -> dict[str, Any]:
    try:
        request_id = request.get("request_id")
        if not isinstance(request_id, str) or not request_id.strip():
            raise SkillFailure(
                "request_id must be a non-empty string",
                error_code="schema_error",
                message_key="media_download.error.invalid_request_id",
            )
        args = _args(request)
        action = _string(args, "action", required=True, max_length=64)
        assert action is not None
        if action not in SUPPORTED_ACTIONS:
            raise SkillFailure(
                f"unsupported action: {action}",
                error_code="unsupported_action",
                message_key="media_download.error.unsupported_action",
            )
        if action == "capabilities":
            return _success(
                request_id,
                action,
                "Media download capabilities are available.",
                _capabilities_extra(),
            )

        output_dir = _artifact_output_directory(request)
        # Downloads and local transcription may legitimately take a long time.
        # Keep per-request network timeouts, cancellation, and durable background
        # polling, but never impose a whole-operation deadline supplied by a
        # planner or an older client for either action.
        operation_timeout = (
            None
            if action in {"download", "transcribe"}
            else _optional_integer(
                args,
                "operation_timeout_seconds",
                minimum=5,
                maximum=2_592_000,
            )
        )
        storage_directory = _skill_storage_directory(request)
        if action == "download":
            command = _build_download_command(
                args,
                output_dir,
                resolve_only=False,
                storage_directory=storage_directory,
            )
        elif action == "resolve":
            command = _build_download_command(args, output_dir, resolve_only=True)
        elif action == "transcribe":
            command = _build_transcribe_command(request, args, output_dir)
        elif action == "ocr":
            command = _build_ocr_command(request, args, output_dir)
        else:
            command = _build_prepare_x_command(request, args, output_dir)
    except SkillFailure as failure:
        raise _mark_not_applied(failure, "pre_dispatch")

    if progress is not None and action != "transcribe":
        progress.emit(
            f"media_download.{action}.starting",
            params={"action": action},
            current=0,
            total=1,
        )

    stdout, stderr, artifacts = _run_tool(
        action,
        command,
        output_dir,
        operation_timeout,
        storage_directory,
        progress.forward_child if progress is not None else None,
    )
    transcription_review = None
    if action == "transcribe":
        transcription_review = _prepare_transcription_review_contract(
            request,
            args,
            artifacts,
        )
    if progress is not None:
        if action == "transcribe":
            total = 2 if _bool(args, "extract_audio_only", False) else 3
            progress.emit(
                "media_download.transcribe.completed",
                params={"action": action},
                current=total,
                total=total,
            )
        else:
            progress.emit(
                f"media_download.{action}.completed",
                params={"action": action},
                current=1,
                total=1,
            )
    if action == "ocr":
        for artifact in artifacts:
            artifact["recognition_source"] = "local_ocr"
            artifact["recognition_engine"] = "tesseract"
        recognition_review = _review_local_ocr_artifact(artifacts)
    else:
        recognition_review = None
    urls = _urls(stdout) if action == "resolve" else []
    if action == "resolve" and not urls:
        raise SkillFailure(
            "media resolver completed without returning a downloadable URL",
            error_code="media_not_found",
            message_key="media_download.error.media_not_found",
            details={"diagnostics": _diagnostics(stderr)},
        )
    delivery_capable = action in {"download", "transcribe", "ocr"}
    deliver_to_user = not delivery_capable or _bool(args, "deliver_to_user", True)
    inline_article = None
    inline_recognition = None
    image_delivery = None
    processing_inputs = None
    if action == "download" and deliver_to_user:
        artifacts, image_delivery, processing_inputs = _package_large_image_delivery(
            artifacts,
            output_dir,
        )
        artifacts, inline_article = _inline_short_article(artifacts)
    elif action == "ocr" and deliver_to_user:
        artifacts, inline_recognition = _inline_short_ocr(artifacts)
    count = len(urls) if action == "resolve" else len(artifacts)
    noun = "URL" if action == "resolve" else "file"
    text = f"{action} completed with {count} {noun}{'' if count == 1 else 's'}."
    if inline_article is not None:
        text = f"{text}\n\n{inline_article['text']}"
    elif inline_recognition is not None:
        text = f"ocr completed with inline text.\n\n{inline_recognition['text']}"
    result_artifacts = artifacts
    delivery = None
    saved_files = None
    if action == "transcribe" and deliver_to_user:
        if _bool(args, "extract_audio_only", False):
            result_artifacts = [
                item
                for item in artifacts
                if item.get("artifact_role") == "extracted_audio"
            ]
            delivery = {"intent": "artifact", "deliver_to_user": True}
        else:
            assert transcription_review is not None
            result_artifacts = []
            saved_files = artifacts
            text = "MEDIA_TRANSCRIPTION_READY"
            delivery = {"intent": "model_synthesis", "deliver_to_user": True}
    elif delivery_capable:
        delivery = {
            "intent": (
                "artifact"
                if deliver_to_user and result_artifacts
                else "model_synthesis"
                if deliver_to_user
                else "save_only"
            ),
            "deliver_to_user": deliver_to_user,
        }
        if not deliver_to_user:
            result_artifacts = []
            saved_files = artifacts
            locations = ", ".join(item["path"] for item in artifacts) or str(output_dir)
            text = f"{action} completed with {count} {noun}{'' if count == 1 else 's'}. Saved locally at: {locations}"
    extra = {
        "count": count,
        "urls": urls,
        "artifacts": result_artifacts,
        "output_directory": str(output_dir),
        "diagnostics": _diagnostics(stderr),
    }
    if action == "download":
        extra["content_bundle"] = _content_bundle(
            artifacts,
            inline_article,
            image_delivery,
        )
        if processing_inputs is not None:
            extra["processing_inputs"] = processing_inputs
        profile_collection = _profile_collection_summary(artifacts)
        if profile_collection is not None:
            extra["profile_collection"] = profile_collection
        if inline_article is not None:
            extra["article_delivery"] = inline_article
    if action == "transcribe" and transcription_review is not None:
        extra["transcription"] = {
            "source": transcription_review["source"],
            "source_engine": transcription_review["source_engine"],
            "target_language": transcription_review["response_language"],
            "raw_character_count": transcription_review["raw_character_count"],
            "reviewed_by_model": False,
            "review_required": True,
        }
        extra["transcription_review"] = transcription_review
    if action == "transcribe" and _bool(args, "extract_audio_only", False):
        extracted_audio = next(
            (
                item
                for item in artifacts
                if item.get("artifact_role") == "extracted_audio"
            ),
            None,
        )
        if extracted_audio is not None:
            extra["processing_outputs"] = {
                "extracted_audio": extracted_audio,
            }
            if not deliver_to_user:
                audio_path = str(extracted_audio["path"])
                extra["followup_policy"] = {
                    "next_action": "preview_transcription",
                    "capability": "audio.preview_transcribe",
                    "input_field": "audio_path",
                    "input_value": audio_path,
                    "fallback_capability": "media_download.transcribe",
                    "fallback_input_field": "input_path",
                    "fallback_input_value": audio_path,
                    "deliver_intermediate": False,
                }
    if delivery is not None:
        extra["delivery"] = delivery
    if saved_files is not None:
        extra["saved_files"] = saved_files
    if action == "ocr":
        extra["recognition"] = {
            "source": "local_ocr",
            "engine": "tesseract",
            "reviewed_by_model": bool(
                recognition_review
                and recognition_review.get("reviewed_by_model") is True
            ),
        }
        if recognition_review is not None:
            extra["recognition_review"] = recognition_review
        if inline_recognition is not None:
            extra["recognition_delivery"] = inline_recognition
    return _success(
        request_id,
        action,
        text,
        extra,
    )


def main() -> None:
    request_id = "invalid"
    progress: ProgressReporter | None = None
    try:
        line = sys.stdin.buffer.readline()
        if not line:
            raise SkillFailure(
                "request line is empty",
                error_code="schema_error",
                message_key="media_download.error.empty_request",
            )
        request = json.loads(line)
        if not isinstance(request, dict):
            raise SkillFailure(
                "request must be a JSON object",
                error_code="schema_error",
                message_key="media_download.error.invalid_request",
            )
        raw_request_id = request.get("request_id")
        if isinstance(raw_request_id, str) and raw_request_id.strip():
            request_id = raw_request_id
        raw_args = request.get("args")
        raw_action = raw_args.get("action") if isinstance(raw_args, dict) else None
        if isinstance(raw_action, str) and raw_action in SUPPORTED_ACTIONS:
            progress = ProgressReporter(request_id)
            progress.emit(
                "media_download.precheck.starting",
                params={"action": raw_action},
                current=0,
                total=1,
            )
        response = respond(request, progress)
    except SkillFailure as failure:
        response = _error(request_id, failure)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        response = _error(
            request_id,
            SkillFailure(
                f"invalid request JSON: {error}",
                error_code="schema_error",
                message_key="media_download.error.invalid_json",
            ),
        )
    except Exception as error:  # protocol boundary
        response = _error(
            request_id,
            SkillFailure(
                f"unexpected media download failure: {error}",
                error_code="execution_failed",
                message_key="media_download.error.unexpected",
            ),
        )
    sys.stdout.write(json.dumps(response, ensure_ascii=False, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
