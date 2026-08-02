from __future__ import annotations

import json
import mimetypes
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
from typing import Any
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
MAX_ARTIFACTS = 32
MAX_DIAGNOSTIC_CHARS = 4_000
INLINE_TEXT_MAX_CHARS = 200
SUBPROCESS_TIMEOUT_SLICE_SECONDS = 24 * 60 * 60
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
    raw = _string(args, "input_path", required=True, max_length=4_096)
    assert raw is not None
    input_path = _input_path(request, raw)
    engine = _choice(args, "engine", ("whisper", "funasr"), "whisper")
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
    if not isinstance(raw_paths, list) or not raw_paths or len(raw_paths) > 32:
        raise SkillFailure(
            "input_paths must be an array containing 1 to 32 image paths",
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

    language = _string(args, "language", default="chi_sim+eng", max_length=64) or "chi_sim+eng"
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
    elif path.suffix.lower() == ".json":
        artifact["artifact_role"] = "metadata"
    return artifact


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
        count_key = roles.get(str(artifact.get("artifact_role") or ""), "other_file_count")
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
    if kind == "video":
        bundle["followup_policy"] = {
            "text_conversion_action": "transcribe_audio",
            "capability": "media_download.transcribe",
            "input_field": "input_path",
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
) -> tuple[str, str, list[dict[str, Any]]]:
    before = _snapshot(output_dir)
    checkpoint_before = _profile_checkpoint_pointers(storage_directory)
    environment = os.environ.copy()
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    if storage_directory is not None:
        environment["MODELSCOPE_CACHE"] = str(storage_directory / "modelscope")
    try:
        completed = _run_process(command, environment, timeout_seconds)
    except subprocess.TimeoutExpired as error:
        stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else (error.stderr or "")
        after = _snapshot(output_dir)
        changed = _changed_artifact_paths(before, after)
        artifacts = [_artifact(path) for path in changed[:MAX_ARTIFACTS]]
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
    artifacts = [_artifact(path) for path in changed[:MAX_ARTIFACTS]]
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
) -> subprocess.CompletedProcess[str]:
    if timeout_seconds is None or timeout_seconds <= SUBPROCESS_TIMEOUT_SLICE_SECONDS:
        return subprocess.run(
            command,
            cwd=TOOL_DIR,
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
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


def _urls(stdout: str) -> list[str]:
    urls: list[str] = []
    for line in stdout.splitlines():
        value = line.strip()
        parsed = urlsplit(value)
        if parsed.scheme in {"http", "https"} and parsed.netloc and value not in urls:
            urls.append(value)
        if len(urls) >= 32:
            break
    return urls


def _funasr_prebuilt_supported(
    system_name: str | None = None,
    machine: str | None = None,
) -> bool:
    system = (system_name or platform.system()).strip().lower()
    architecture = (machine or platform.machine()).strip().lower()
    return not (system == "darwin" and architecture in {"x86_64", "amd64"})


def _capabilities_extra() -> dict[str, Any]:
    funasr_supported = _funasr_prebuilt_supported()
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
            "ocr_is_separate": True,
        },
        "installed_dependencies": {
            "youtube": ["yt-dlp"],
            "media_processing": ["ffmpeg", "ffprobe"],
            "ocr": ["tesseract", "chi_sim"],
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


def _emit_precheck_progress(request_id: str, action: str) -> None:
    frame = {
        "schema_version": 1,
        "record_type": "skill_progress",
        "request_id": request_id,
        "sequence": 1,
        "kind": "progress",
        "detail_key": "media_download.precheck.starting",
        "params": {"action": action},
        "current": 0,
        "total": 1,
    }
    sys.stdout.write(json.dumps(frame, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def respond(request: dict[str, Any]) -> dict[str, Any]:
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
        # Downloads may legitimately take a long time for large files or slow
        # links.  Keep their per-request network timeout, cancellation, and
        # durable background polling, but never impose a whole-operation
        # deadline supplied by a planner or an older client.
        operation_timeout = (
            None
            if action == "download"
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

    stdout, stderr, artifacts = _run_tool(
        action,
        command,
        output_dir,
        operation_timeout,
        storage_directory,
    )
    if action == "ocr":
        for artifact in artifacts:
            artifact["recognition_source"] = "local_ocr"
            artifact["recognition_engine"] = "tesseract"
    urls = _urls(stdout) if action == "resolve" else []
    if action == "resolve" and not urls:
        raise SkillFailure(
            "media resolver completed without returning a downloadable URL",
            error_code="media_not_found",
            message_key="media_download.error.media_not_found",
            details={"diagnostics": _diagnostics(stderr)},
        )
    delivery_capable = action in {"download", "ocr"}
    deliver_to_user = not delivery_capable or _bool(args, "deliver_to_user", True)
    inline_article = None
    inline_recognition = None
    if action == "download" and deliver_to_user:
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
    if delivery_capable:
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
        extra["content_bundle"] = _content_bundle(artifacts, inline_article)
        profile_collection = _profile_collection_summary(artifacts)
        if profile_collection is not None:
            extra["profile_collection"] = profile_collection
        if inline_article is not None:
            extra["article_delivery"] = inline_article
    if delivery is not None:
        extra["delivery"] = delivery
    if saved_files is not None:
        extra["saved_files"] = saved_files
    if action == "ocr":
        extra["recognition"] = {
            "source": "local_ocr",
            "engine": "tesseract",
        }
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
            _emit_precheck_progress(request_id, raw_action)
        response = respond(request)
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
