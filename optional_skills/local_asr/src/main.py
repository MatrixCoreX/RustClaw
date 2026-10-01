from __future__ import annotations

import json
import mimetypes
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from typing import Any


SKILL_NAME = "local_asr"
SCHEMA_VERSION = 1
SUPPORTED_ACTIONS = ("capabilities", "transcribe")
SUPPORTED_ENGINES = ("whisper", "funasr")
DEFAULT_FUNASR_MODEL = "iic/SenseVoiceSmall"
DEFAULT_FUNASR_VAD_MODEL = "fsmn-vad"
DEFAULT_SEGMENT_SECONDS = 480
SEGMENT_SIZE_FALLBACK_BYTES = 64 * 1024 * 1024
FUNASR_MODEL_PATHS = {
    DEFAULT_FUNASR_MODEL: ("iic", "SenseVoiceSmall"),
    DEFAULT_FUNASR_VAD_MODEL: ("iic", "speech_fsmn_vad_zh-cn-16k-common-pytorch"),
    "iic/speech_fsmn_vad_zh-cn-16k-common-pytorch": (
        "iic",
        "speech_fsmn_vad_zh-cn-16k-common-pytorch",
    ),
}
FUNASR_TAG_RE = re.compile(r"<\|[^|>]+?\|>")
FUNASR_MARKER_RE = re.compile(
    "[\U0001f600\U0001f604\U0001f60a\U0001f614\U0001f621"
    "\U0001f630\U0001f62e\U0001f922\U0001f927\U0001f62d"
    "\U0001f637\U0001f3bc\U0001f44f\u2753]"
)


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

    def emit(self, detail_key: str, current: int, total: int) -> None:
        self.sequence += 1
        frame = {
            "schema_version": 1,
            "record_type": "skill_progress",
            "request_id": self.request_id,
            "sequence": self.sequence,
            "kind": "progress",
            "detail_key": detail_key,
            "params": {
                "step_id": "extract_audio" if current == 1 else "transcribe_speech",
                "step_status": "completed" if current == total else "in_progress",
            },
            "current": current,
            "total": total,
        }
        print(json.dumps(frame, ensure_ascii=False, separators=(",", ":")), flush=True)


def _args(request: dict[str, Any]) -> dict[str, Any]:
    args = request.get("args")
    if not isinstance(args, dict):
        raise SkillFailure(
            "args must be an object",
            error_code="invalid_args",
            message_key="local_asr.error.invalid_args",
        )
    return args


def _string(
    args: dict[str, Any],
    name: str,
    *,
    required: bool = False,
    default: str | None = None,
    max_length: int = 4096,
) -> str | None:
    value = args.get(name, default)
    if value is None:
        if required:
            raise SkillFailure(
                f"missing required argument: {name}",
                error_code="missing_argument",
                message_key=f"local_asr.error.missing_{name}",
            )
        return None
    if not isinstance(value, str) or not value.strip() or "\x00" in value:
        raise SkillFailure(
            f"{name} must be a non-empty string",
            error_code="invalid_args",
            message_key=f"local_asr.error.invalid_{name}",
        )
    value = value.strip()
    if len(value) > max_length:
        raise SkillFailure(
            f"{name} is too long",
            error_code="invalid_args",
            message_key=f"local_asr.error.invalid_{name}",
        )
    return value


def _bool(args: dict[str, Any], name: str, default: bool = False) -> bool:
    value = args.get(name, default)
    if not isinstance(value, bool):
        raise SkillFailure(
            f"{name} must be a boolean",
            error_code="invalid_args",
            message_key=f"local_asr.error.invalid_{name}",
        )
    return value


def _integer(
    args: dict[str, Any], name: str, default: int, minimum: int, maximum: int
) -> int:
    value = args.get(name, default)
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise SkillFailure(
            f"{name} must be an integer between {minimum} and {maximum}",
            error_code="invalid_args",
            message_key=f"local_asr.error.invalid_{name}",
        )
    return value


def _context_directory(request: dict[str, Any], field: str) -> Path:
    context = request.get("context")
    raw = context.get(field) if isinstance(context, dict) else None
    if not isinstance(raw, str) or not raw.strip():
        raise SkillFailure(
            f"runtime {field} is unavailable",
            error_code="invalid_args",
            message_key=f"local_asr.error.{field}_unavailable",
        )
    path = Path(raw).expanduser().resolve()
    path.mkdir(parents=True, exist_ok=True)
    return path


def _storage_directory(request: dict[str, Any]) -> Path:
    context = request.get("context")
    storage = context.get("skill_storage") if isinstance(context, dict) else None
    raw = storage.get("directory_path") if isinstance(storage, dict) else None
    if (
        not isinstance(storage, dict)
        or storage.get("storage_kind") != "directory"
        or not isinstance(raw, str)
        or not raw.strip()
    ):
        raise SkillFailure(
            "skill storage directory is unavailable",
            error_code="invalid_args",
            message_key="local_asr.error.storage_unavailable",
        )
    path = Path(raw).expanduser()
    if not path.is_absolute():
        raise SkillFailure(
            "skill storage directory must be absolute",
            error_code="invalid_args",
            message_key="local_asr.error.storage_unavailable",
        )
    return path.resolve()


def _input_path(request: dict[str, Any], raw: str) -> Path:
    context = request.get("context")
    workspace = context.get("workspace_root") if isinstance(context, dict) else None
    workspace_root = (
        Path(workspace).expanduser().resolve()
        if isinstance(workspace, str) and workspace.strip()
        else None
    )
    permissions = context.get("permissions") if isinstance(context, dict) else None
    allow_outside = (
        isinstance(permissions, dict)
        and permissions.get("allow_path_outside_workspace") is True
    )
    path = Path(raw).expanduser()
    if not path.is_absolute() and workspace_root is not None:
        path = workspace_root / path
    path = path.resolve()
    if not path.is_file():
        raise SkillFailure(
            f"input file does not exist: {path}",
            error_code="not_found",
            message_key="local_asr.error.input_not_found",
        )
    if workspace_root is not None and not allow_outside:
        try:
            path.relative_to(workspace_root)
        except ValueError as error:
            raise SkillFailure(
                f"input path is outside the allowed workspace: {path}",
                error_code="permission_denied",
                message_key="local_asr.error.path_outside_workspace",
            ) from error
    return path


def _find_executable(explicit: str | None, environment_names: tuple[str, ...], names: tuple[str, ...]) -> Path:
    candidates = [explicit] if explicit else []
    candidates.extend(os.environ.get(name) for name in environment_names)
    candidates.extend(names)
    for candidate in candidates:
        if not candidate:
            continue
        path = Path(candidate).expanduser()
        if path.is_file():
            return path.resolve()
        found = shutil.which(candidate)
        if found:
            return Path(found).resolve()
    raise SkillFailure(
        "local ASR executable is unavailable",
        error_code="dependency_unavailable",
        message_key="local_asr.error.executable_unavailable",
    )


def _find_whisper_model(explicit: str | None) -> Path:
    candidates = [explicit] if explicit else []
    candidates.extend(
        os.environ.get(name)
        for name in ("WHISPER_MODEL", "WHISPER_MODEL_PATH", "WHISPER_CPP_MODEL")
    )
    for candidate in candidates:
        if candidate and Path(candidate).expanduser().is_file():
            return Path(candidate).expanduser().resolve()
    raise SkillFailure(
        "whisper.cpp model is unavailable",
        error_code="dependency_unavailable",
        message_key="local_asr.error.model_unavailable",
    )


def _run(command: list[str]) -> None:
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except OSError as error:
        raise SkillFailure(
            f"local ASR process could not start: {error}",
            error_code="dependency_unavailable",
            message_key="local_asr.error.process_unavailable",
        ) from error
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "").strip()[-4000:]
        raise SkillFailure(
            detail or f"local ASR process exited with code {completed.returncode}",
            error_code="execution_failed",
            message_key="local_asr.error.execution_failed",
            retryable=False,
            details={"exit_code": completed.returncode},
        )


def _extract_audio(input_path: Path, output_dir: Path) -> Path:
    if input_path.suffix.lower() == ".wav":
        return input_path
    ffmpeg = _find_executable(None, (), ("ffmpeg",))
    output = output_dir / f"{input_path.stem}_audio.wav"
    _run(
        [
            str(ffmpeg),
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-i",
            str(input_path),
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            str(output),
        ]
    )
    if not output.is_file() or output.stat().st_size == 0:
        raise SkillFailure(
            "audio extraction produced no output",
            error_code="execution_failed",
            message_key="local_asr.error.audio_extraction_failed",
        )
    return output


def _probe_duration_seconds(input_path: Path) -> float | None:
    try:
        ffprobe = _find_executable(None, (), ("ffprobe",))
    except SkillFailure:
        return None
    try:
        completed = subprocess.run(
            [
                str(ffprobe),
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                str(input_path),
            ],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    try:
        duration = float(completed.stdout.strip())
    except ValueError:
        return None
    return duration if duration > 0 else None


def _requires_segmentation(input_path: Path, duration_seconds: float | None) -> bool:
    if duration_seconds is not None:
        return duration_seconds > DEFAULT_SEGMENT_SECONDS
    try:
        return input_path.stat().st_size > SEGMENT_SIZE_FALLBACK_BYTES
    except OSError:
        return False


def _split_audio(input_path: Path, output_dir: Path) -> list[Path]:
    ffmpeg = _find_executable(None, (), ("ffmpeg",))
    output_pattern = output_dir / "segment_%05d.wav"
    _run(
        [
            str(ffmpeg),
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-i",
            str(input_path),
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            "-f",
            "segment",
            "-segment_time",
            str(DEFAULT_SEGMENT_SECONDS),
            "-reset_timestamps",
            "1",
            str(output_pattern),
        ]
    )
    segments = sorted(output_dir.glob("segment_*.wav"))
    if not segments or any(path.stat().st_size == 0 for path in segments):
        raise SkillFailure(
            "audio segmentation produced no usable output",
            error_code="execution_failed",
            message_key="local_asr.error.audio_segmentation_failed",
        )
    return segments


def _simplify_chinese(text: str) -> str:
    try:
        from opencc import OpenCC
    except ImportError as error:
        raise SkillFailure(
            "OpenCC is unavailable",
            error_code="dependency_unavailable",
            message_key="local_asr.error.opencc_unavailable",
        ) from error
    return OpenCC("tw2sp").convert(text)


def _resolve_funasr_model(storage: Path, name: str) -> str:
    explicit = Path(name).expanduser()
    if explicit.is_dir():
        return str(explicit.resolve())
    relative = FUNASR_MODEL_PATHS.get(name)
    if relative is None:
        return name
    candidate = storage.joinpath("modelscope", *relative)
    if candidate.is_dir() and (candidate / "model.pt").is_file() and (
        (candidate / "config.yaml").is_file()
        or (candidate / "configuration.json").is_file()
    ):
        return str(candidate)
    raise SkillFailure(
        f"installed local ASR model is incomplete: {candidate}",
        error_code="dependency_unavailable",
        message_key="local_asr.error.model_incomplete",
    )


def _funasr_text(value: object) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        return value.get("text") if isinstance(value.get("text"), str) else ""
    if isinstance(value, list):
        return "\n".join(part for item in value if (part := _funasr_text(item)))
    return ""


def _create_funasr_recognizer(storage: Path, args: dict[str, Any]) -> Any:
    try:
        from funasr import AutoModel
    except ImportError as error:
        raise SkillFailure(
            "FunASR is unavailable",
            error_code="dependency_unavailable",
            message_key="local_asr.error.funasr_unavailable",
        ) from error
    model_name = _string(args, "funasr_model", default=DEFAULT_FUNASR_MODEL, max_length=512)
    vad_name = _string(args, "funasr_vad_model", default=DEFAULT_FUNASR_VAD_MODEL, max_length=512)
    assert model_name is not None and vad_name is not None
    try:
        return AutoModel(
            model=_resolve_funasr_model(storage, model_name),
            vad_model=_resolve_funasr_model(storage, vad_name),
            vad_kwargs={"check_latest": False},
            device="cpu",
            disable_update=True,
            check_latest=False,
        )
    except SkillFailure:
        raise
    except Exception as error:
        raise SkillFailure(
            f"FunASR initialization failed: {error}",
            error_code="execution_failed",
            message_key="local_asr.error.execution_failed",
        ) from error


def _transcribe_with_funasr_recognizer(
    recognizer: Any,
    audio: Path,
    output: Path,
    args: dict[str, Any],
) -> None:
    try:
        result = recognizer.generate(
            input=str(audio),
            batch_size_s=_integer(args, "funasr_batch_size_s", 60, 1, 600),
            use_itn=True,
        )
    except SkillFailure:
        raise
    except Exception as error:
        raise SkillFailure(
            f"FunASR failed: {error}",
            error_code="execution_failed",
            message_key="local_asr.error.execution_failed",
        ) from error
    text = FUNASR_TAG_RE.sub("", _funasr_text(result)).strip()
    if not _bool(args, "rich_text", False):
        text = FUNASR_MARKER_RE.sub("", text).strip()
    if not text:
        raise SkillFailure(
            "FunASR returned no transcript text",
            error_code="transcript_empty",
            message_key="local_asr.error.transcript_empty",
        )
    output.write_text(text + "\n", encoding="utf-8")


def _transcribe_funasr(audio: Path, output: Path, storage: Path, args: dict[str, Any]) -> None:
    _transcribe_with_funasr_recognizer(
        _create_funasr_recognizer(storage, args),
        audio,
        output,
        args,
    )


def _transcribe_whisper(audio: Path, output: Path, args: dict[str, Any]) -> None:
    binary = _find_executable(
        _string(args, "whisper_bin", max_length=4096),
        ("WHISPER_BIN", "WHISPER_CPP_BIN", "WHISPER_CLI"),
        ("whisper-cli", "whisper.cpp"),
    )
    model = _find_whisper_model(_string(args, "whisper_model", max_length=4096))
    prefix = output.with_suffix("")
    command = [
        str(binary),
        "-m",
        str(model),
        "-f",
        str(audio),
        "-l",
        _string(args, "language", default="auto", max_length=64) or "auto",
        "-otxt",
        "-of",
        str(prefix),
        "-t",
        str(_integer(args, "threads", min(os.cpu_count() or 2, 8), 1, 64)),
        "--no-timestamps",
    ]
    if _bool(args, "translate", False):
        command.append("--translate")
    if _bool(args, "no_gpu", False):
        command.append("--no-gpu")
    if _bool(args, "fast", False):
        command.extend(["--best-of", "1", "--beam-size", "1", "--no-fallback"])
    _run(command)
    generated = prefix.with_suffix(".txt")
    if generated != output and generated.is_file():
        generated.replace(output)
    if not output.is_file():
        raise SkillFailure(
            "whisper.cpp produced no transcript text",
            error_code="transcript_missing",
            message_key="local_asr.error.transcript_missing",
        )


def _target_language(request: dict[str, Any], args: dict[str, Any]) -> str:
    explicit = args.get("response_language")
    if isinstance(explicit, str) and explicit.strip():
        return explicit.strip()
    context = request.get("context")
    if isinstance(context, dict):
        for key in ("locale", "language"):
            value = context.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
    return "preserve-source-language"


def _requires_simplified_chinese(language: str) -> bool:
    normalized = language.strip().replace("_", "-").lower()
    return normalized in {"zh", "zh-cn", "zh-sg", "zh-hans"} or normalized.startswith("zh-hans-")


def _artifact(path: Path, role: str) -> dict[str, Any]:
    mime_type, _ = mimetypes.guess_type(path.name)
    return {
        "path": str(path),
        "filename": path.name,
        "mime_type": mime_type or "application/octet-stream",
        "size_bytes": path.stat().st_size,
        "artifact_role": role,
    }


def _success(request_id: str, action: str, extra: dict[str, Any]) -> dict[str, Any]:
    return {
        "request_id": request_id,
        "status": "ok",
        "text": "LOCAL_ASR_READY",
        "error_text": None,
        "extra": {
            "schema_version": SCHEMA_VERSION,
            "source_skill": SKILL_NAME,
            "status": "ok",
            "action": action,
            **extra,
        },
    }


def _error(request_id: str, failure: SkillFailure) -> dict[str, Any]:
    return {
        "request_id": request_id,
        "status": "error",
        "text": "",
        "error_text": str(failure),
        "extra": {
            "schema_version": SCHEMA_VERSION,
            "source_skill": SKILL_NAME,
            "status": "error",
            "error_code": failure.error_code,
            "message_key": failure.message_key,
            "retryable": failure.retryable,
            **failure.details,
        },
    }


def respond(request: dict[str, Any], progress: ProgressReporter | None = None) -> dict[str, Any]:
    request_id = request.get("request_id")
    if not isinstance(request_id, str) or not request_id.strip():
        raise SkillFailure(
            "request_id must be a non-empty string",
            error_code="schema_error",
            message_key="local_asr.error.invalid_request_id",
        )
    args = _args(request)
    action = _string(args, "action", required=True, max_length=64)
    if action not in SUPPORTED_ACTIONS:
        raise SkillFailure(
            f"unsupported action: {action}",
            error_code="unsupported_action",
            message_key="local_asr.error.unsupported_action",
        )
    if action == "capabilities":
        return _success(
            request_id,
            action,
            {
                "engines": list(SUPPORTED_ENGINES),
                "model_lifecycle": "per_invocation_process",
                "runtime_network": False,
            },
        )

    input_path = _input_path(
        request,
        _string(args, "input_path", required=True, max_length=4096) or "",
    )
    output_dir = _context_directory(request, "artifact_output_directory")
    storage = _storage_directory(request)
    engine = _string(args, "engine", default="whisper", max_length=32) or "whisper"
    if engine not in SUPPORTED_ENGINES:
        raise SkillFailure(
            f"unsupported local ASR engine: {engine}",
            error_code="invalid_args",
            message_key="local_asr.error.invalid_engine",
        )
    duration_seconds = _probe_duration_seconds(input_path)
    segmented = _requires_segmentation(input_path, duration_seconds)
    transcript = output_dir / f"{input_path.stem}_transcript.txt"
    extracted_audio: Path | None = None
    if progress is not None:
        progress.emit("local_asr.extracting_audio", 1, 3)
    with tempfile.TemporaryDirectory(prefix=".local-asr-", dir=output_dir) as temporary:
        if segmented:
            audio_inputs = _split_audio(input_path, Path(temporary))
        else:
            extracted_audio = _extract_audio(input_path, output_dir)
            audio_inputs = [extracted_audio]
        if progress is not None:
            progress.emit("local_asr.recognizing_speech", 2, 3)
        funasr_recognizer = (
            _create_funasr_recognizer(storage, args) if engine == "funasr" else None
        )
        transcript_parts: list[str] = []
        for index, audio_input in enumerate(audio_inputs):
            part_output = Path(temporary) / f"transcript_{index:05d}.txt"
            if funasr_recognizer is not None:
                _transcribe_with_funasr_recognizer(
                    funasr_recognizer,
                    audio_input,
                    part_output,
                    args,
                )
            else:
                _transcribe_whisper(audio_input, part_output, args)
            part = part_output.read_text(encoding="utf-8").strip()
            if part:
                transcript_parts.append(part)
        transcript.write_text("\n".join(transcript_parts) + "\n", encoding="utf-8")
    raw_text = transcript.read_text(encoding="utf-8").strip()
    if not raw_text:
        raise SkillFailure(
            "local ASR produced no transcript text",
            error_code="transcript_empty",
            message_key="local_asr.error.transcript_empty",
        )
    target_language = _target_language(request, args)
    if _requires_simplified_chinese(target_language):
        raw_text = _simplify_chinese(raw_text)
        transcript.write_text(raw_text + "\n", encoding="utf-8")
    if progress is not None:
        progress.emit("local_asr.completed", 3, 3)
    transcript_artifact = _artifact(transcript, "transcript_text")
    saved_files = [transcript_artifact]
    if extracted_audio is not None and extracted_audio != input_path:
        saved_files.insert(0, _artifact(extracted_audio, "extracted_audio"))
    return _success(
        request_id,
        action,
        {
            "engine": engine,
            "artifacts": [],
            "saved_files": saved_files,
            "delivery": {"intent": "model_synthesis", "deliver_to_user": True},
            "transcription": {
                "source": "local_asr",
                "source_engine": engine,
                "target_language": target_language,
                "raw_character_count": len(raw_text),
                "reviewed_by_model": False,
                "review_required": True,
            },
            "chunking": {
                "segmented": segmented,
                "segment_seconds": DEFAULT_SEGMENT_SECONDS,
                "segment_count": len(audio_inputs),
                "source_duration_seconds": duration_seconds,
                "merge_order": "source_order",
            },
            "transcription_review": {
                "schema_version": 1,
                "required": True,
                "source": "local_asr",
                "source_engine": engine,
                "raw_text": raw_text,
                "raw_character_count": len(raw_text),
                "response_language": target_language,
                "corrections": ["recognition_errors", "typos", "broken_sentences"],
                "preserve_meaning": True,
                "delivery": {
                    "mode": "inline_and_artifact",
                    "text_format": "text/plain; charset=utf-8",
                    "text_filename": transcript.name,
                },
            },
        },
    )


def main() -> None:
    request_id = "invalid"
    try:
        line = sys.stdin.buffer.readline()
        if not line:
            raise SkillFailure(
                "request line is empty",
                error_code="schema_error",
                message_key="local_asr.error.empty_request",
            )
        request = json.loads(line)
        if not isinstance(request, dict):
            raise SkillFailure(
                "request must be a JSON object",
                error_code="schema_error",
                message_key="local_asr.error.invalid_request",
            )
        raw_request_id = request.get("request_id")
        if isinstance(raw_request_id, str) and raw_request_id.strip():
            request_id = raw_request_id
        progress = ProgressReporter(request_id)
        response = respond(request, progress)
    except SkillFailure as failure:
        response = _error(request_id, failure)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        response = _error(
            request_id,
            SkillFailure(
                f"invalid request JSON: {error}",
                error_code="schema_error",
                message_key="local_asr.error.invalid_json",
            ),
        )
    except Exception as error:
        response = _error(
            request_id,
            SkillFailure(
                f"unexpected local ASR failure: {error}",
                error_code="execution_failed",
                message_key="local_asr.error.unexpected",
            ),
        )
    print(json.dumps(response, ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    main()
