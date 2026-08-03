#!/usr/bin/env python3
"""
Extract audio from a local media file and transcribe it with a local ASR engine.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Iterable

from task_cancellation import CancellationToken, OperationCancelled


DEFAULT_DOWNLOAD_DIR = "downloads"
DEFAULT_AUDIO_SUFFIX = "_audio"
DEFAULT_TRANSCRIPT_SUFFIX = "_transcript"
DEFAULT_SAMPLE_RATE = 16000
DEFAULT_CHANNELS = 1
DEFAULT_LANGUAGE = "auto"
DEFAULT_MAX_THREADS = 8
DEFAULT_TRANSCRIBE_ENGINE = "whisper"
TRANSCRIBE_ENGINES = ("whisper", "funasr")
PARENT_PROGRESS_PREFIX = "__MEDIA_DOWNLOAD_PROGRESS__:"
DEFAULT_FUNASR_MODEL = "iic/SenseVoiceSmall"
DEFAULT_FUNASR_DEVICE = "cpu"
DEFAULT_FUNASR_VAD_MODEL = "fsmn-vad"
DEFAULT_FUNASR_PUNC_MODEL = None
DEFAULT_FUNASR_BATCH_SIZE_S = 60
DEFAULT_FUNASR_RICH_TEXT = False
DEFAULT_SIMPLIFY_CHINESE = True
FUNASR_MANAGED_MODEL_PATHS = {
    DEFAULT_FUNASR_MODEL: ("iic", "SenseVoiceSmall"),
    DEFAULT_FUNASR_VAD_MODEL: ("iic", "speech_fsmn_vad_zh-cn-16k-common-pytorch"),
    "iic/speech_fsmn_vad_zh-cn-16k-common-pytorch": (
        "iic",
        "speech_fsmn_vad_zh-cn-16k-common-pytorch",
    ),
}
FUNASR_RICH_TAG_RE = re.compile(r"<\|[^|>]+?\|>")
FUNASR_RICH_MARKER_RE = re.compile(
    "["
    "\U0001f600"
    "\U0001f604"
    "\U0001f60a"
    "\U0001f614"
    "\U0001f621"
    "\U0001f630"
    "\U0001f62e"
    "\U0001f922"
    "\U0001f927"
    "\U0001f62d"
    "\U0001f637"
    "\U0001f3bc"
    "\U0001f44f"
    "\u2753"
    "]"
)
MEDIA_EXTENSIONS = {
    ".mp4",
    ".mov",
    ".m4v",
    ".mkv",
    ".webm",
    ".avi",
    ".flv",
    ".mp3",
    ".m4a",
    ".aac",
    ".wav",
    ".flac",
    ".ogg",
}
WHISPER_BIN_ENV = ("WHISPER_BIN", "WHISPER_CPP_BIN", "WHISPER_CLI")
WHISPER_MODEL_ENV = ("WHISPER_MODEL", "WHISPER_MODEL_PATH", "WHISPER_CPP_MODEL")
WHISPER_PROGRESS_RE = re.compile(r"progress\s*=\s*(-?\d+)%")


class VideoTranscribeError(RuntimeError):
    """Raised when audio extraction or transcription cannot be completed."""


def emit_parent_progress(detail_key: str, current: int, total: int) -> None:
    payload = {
        "detail_key": detail_key,
        "current": current,
        "total": total,
    }
    print(
        PARENT_PROGRESS_PREFIX
        + json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
        file=sys.stderr,
        flush=True,
    )


def require_binary(name: str) -> str:
    binary = shutil.which(name)
    if not binary:
        raise VideoTranscribeError(f"{name} is required but was not found in PATH.")
    return binary


def default_whisper_threads() -> int:
    cpu_count = os.cpu_count() or 4
    return max(1, min(cpu_count, DEFAULT_MAX_THREADS))


def latest_media(directory: Path) -> Path:
    if not directory.exists():
        raise VideoTranscribeError(f"Directory does not exist: {directory}")
    candidates = [
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix.lower() in MEDIA_EXTENSIONS
    ]
    preferred_candidates = [path for path in candidates if not is_generated_audio_output(path)]
    if preferred_candidates:
        candidates = preferred_candidates
    if not candidates:
        raise VideoTranscribeError(f"No media files found in {directory}")
    return max(candidates, key=lambda path: path.stat().st_mtime)


def path_from_env(names: Iterable[str]) -> Path | None:
    for name in names:
        value = os.environ.get(name)
        if value:
            return Path(value).expanduser()
    return None


def find_whisper_binary(explicit: str | None = None) -> Path:
    if explicit:
        path = Path(explicit).expanduser()
        if path.exists():
            return path
        found = shutil.which(explicit)
        if found:
            return Path(found)
        raise VideoTranscribeError(f"whisper.cpp binary was not found: {explicit}")

    env_path = path_from_env(WHISPER_BIN_ENV)
    if env_path:
        if env_path.exists():
            return env_path
        found = shutil.which(str(env_path))
        if found:
            return Path(found)
        raise VideoTranscribeError(f"whisper.cpp binary from environment was not found: {env_path}")

    for executable in ("whisper-cli", "whisper.cpp"):
        found = shutil.which(executable)
        if found:
            return Path(found)

    raise VideoTranscribeError(
        "whisper.cpp binary was not found. Pass --whisper-bin or set WHISPER_BIN."
    )


def find_whisper_model(explicit: str | None = None) -> Path:
    if explicit:
        path = Path(explicit).expanduser()
        if path.exists():
            return path
        raise VideoTranscribeError(f"whisper.cpp model was not found: {explicit}")

    env_path = path_from_env(WHISPER_MODEL_ENV)
    if env_path:
        if env_path.exists():
            return env_path
        raise VideoTranscribeError(f"whisper.cpp model from environment was not found: {env_path}")

    raise VideoTranscribeError(
        "whisper.cpp model was not found. Pass --model or set WHISPER_MODEL."
    )


def output_path_for(
    input_path: Path,
    output: str | None,
    output_dir: str | None,
    suffix: str,
    extension: str,
) -> Path:
    if output:
        path = Path(output).expanduser()
        if path.suffix.lower() != extension:
            path = path.with_suffix(extension)
        return path

    parent = Path(output_dir).expanduser() if output_dir else input_path.parent
    return parent / f"{input_path.stem}{suffix}{extension}"


def is_generated_audio_output(path: Path) -> bool:
    if path.suffix.lower() != ".wav" or not path.stem.endswith(DEFAULT_AUDIO_SUFFIX):
        return False
    source_stem = path.stem[: -len(DEFAULT_AUDIO_SUFFIX)]
    if not source_stem:
        return False
    return any((path.parent / f"{source_stem}{extension}").exists() for extension in MEDIA_EXTENSIONS)


def transcript_stem_for(input_path: Path) -> str:
    stem = input_path.stem
    if input_path.suffix.lower() == ".wav" and stem.endswith(DEFAULT_AUDIO_SUFFIX):
        stripped = stem[: -len(DEFAULT_AUDIO_SUFFIX)]
        if stripped:
            return stripped
    return stem


def transcript_output_path_for(input_path: Path, output: str | None, output_dir: str | None) -> Path:
    if output:
        return output_path_for(input_path, output, output_dir, DEFAULT_TRANSCRIPT_SUFFIX, ".txt")
    parent = Path(output_dir).expanduser() if output_dir else input_path.parent
    return parent / f"{transcript_stem_for(input_path)}{DEFAULT_TRANSCRIPT_SUFFIX}.txt"


def transcript_prefix_for(transcript_path: Path) -> Path:
    if transcript_path.suffix.lower() == ".txt":
        return transcript_path.with_suffix("")
    return transcript_path


def build_extract_audio_command(
    ffmpeg: str,
    input_path: Path,
    audio_path: Path,
    *,
    overwrite: bool,
    sample_rate: int,
    channels: int,
) -> list[str]:
    return [
        ffmpeg,
        "-hide_banner",
        "-y" if overwrite else "-n",
        "-i",
        str(input_path),
        "-map",
        "0:a:0",
        "-vn",
        "-ac",
        str(channels),
        "-ar",
        str(sample_rate),
        "-c:a",
        "pcm_s16le",
        str(audio_path),
    ]


def probe_audio_stream(input_path: Path) -> bool | None:
    """Return whether ffprobe finds an audio stream, or None when probing is unavailable."""
    ffprobe = shutil.which("ffprobe")
    if not ffprobe:
        return None
    try:
        completed = subprocess.run(
            [
                ffprobe,
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=index",
                "-of",
                "csv=p=0",
                str(input_path),
            ],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    return bool(completed.stdout.strip())


def render_progress_bar(percent: int, *, width: int = 30) -> str:
    clamped = max(0, min(percent, 100))
    filled = round(width * clamped / 100)
    return f"transcribe_progress: [{'#' * filled}{'.' * (width - filled)}] {clamped:3d}%"


def print_progress_bar(percent: int, *, interactive: bool) -> None:
    line = render_progress_bar(percent)
    if interactive:
        progress_writer = getattr(sys.stderr, "write_progress", None)
        if callable(progress_writer):
            progress_writer(line)
            return
        print(f"\r{line}", end="", file=sys.stderr, flush=True)
        return
    print(line, file=sys.stderr, flush=True)


def finish_progress_bar(*, interactive: bool) -> None:
    if interactive:
        progress_finisher = getattr(sys.stderr, "finish_progress", None)
        if callable(progress_finisher):
            progress_finisher()
            return
        print(file=sys.stderr, flush=True)


def run_streaming_command(
    command: list[str],
    *,
    verbose: bool,
    cancel_token: CancellationToken | None = None,
) -> subprocess.CompletedProcess[str]:
    output_parts: list[str] = []
    last_progress = 0
    interactive_progress = sys.stderr.isatty()
    print_progress_bar(0, interactive=interactive_progress)
    if cancel_token is not None:
        cancel_token.raise_if_cancelled()
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
        start_new_session=os.name == "posix",
    )
    try:
        if cancel_token is not None:
            cancel_token.register_process(process)
        assert process.stdout is not None
        for line in process.stdout:
            output_parts.append(line)
            match = WHISPER_PROGRESS_RE.search(line)
            if match:
                progress = max(0, min(int(match.group(1)), 100))
                if progress != last_progress:
                    print_progress_bar(progress, interactive=interactive_progress)
                    last_progress = progress
                continue
            if verbose:
                print(line, end="", file=sys.stderr, flush=True)

        returncode = process.wait()
        if cancel_token is not None:
            cancel_token.raise_if_cancelled()
        if returncode == 0 and last_progress < 100:
            print_progress_bar(100, interactive=interactive_progress)
    finally:
        if cancel_token is not None:
            cancel_token.unregister_process(process)
        finish_progress_bar(interactive=interactive_progress)
    return subprocess.CompletedProcess(command, returncode, "", "".join(output_parts))


def run_command(
    command: list[str],
    *,
    verbose: bool,
    stream_output: bool = False,
    cancel_token: CancellationToken | None = None,
) -> subprocess.CompletedProcess[str]:
    if verbose:
        print(" ".join(command), file=sys.stderr)
    if stream_output:
        return run_streaming_command(command, verbose=verbose, cancel_token=cancel_token)
    if cancel_token is not None:
        cancel_token.raise_if_cancelled()
    process = subprocess.Popen(
        command,
        stdout=None if verbose else subprocess.PIPE,
        stderr=None if verbose else subprocess.PIPE,
        text=True,
        start_new_session=os.name == "posix",
    )
    try:
        if cancel_token is not None:
            cancel_token.register_process(process)
        stdout, stderr = process.communicate()
        if cancel_token is not None:
            cancel_token.raise_if_cancelled()
    finally:
        if cancel_token is not None:
            cancel_token.unregister_process(process)
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def command_error(command_name: str, completed: subprocess.CompletedProcess[str]) -> VideoTranscribeError:
    stderr = (completed.stderr or "").strip()
    detail = f": {stderr}" if stderr else ""
    return VideoTranscribeError(f"{command_name} failed with exit code {completed.returncode}{detail}")


def prepare_transcript_output(transcript_path: Path, *, overwrite: bool) -> None:
    if transcript_path.exists():
        if not overwrite:
            raise VideoTranscribeError(
                f"Transcript output already exists, pass --overwrite to replace it: {transcript_path}"
            )
        transcript_path.unlink()
    transcript_path.parent.mkdir(parents=True, exist_ok=True)


def extract_audio(
    input_path: Path,
    audio_path: Path,
    *,
    overwrite: bool = False,
    reuse_audio: bool = False,
    sample_rate: int = DEFAULT_SAMPLE_RATE,
    channels: int = DEFAULT_CHANNELS,
    cancel_token: CancellationToken | None = None,
    verbose: bool = False,
) -> Path:
    if not input_path.exists():
        raise VideoTranscribeError(f"Input file does not exist: {input_path}")
    if audio_path.exists():
        if reuse_audio:
            return audio_path
        if not overwrite:
            raise VideoTranscribeError(f"Audio output already exists, pass --overwrite to replace it: {audio_path}")
    if probe_audio_stream(input_path) is False:
        raise VideoTranscribeError(f"Input media contains no audio stream: {input_path}")

    audio_path.parent.mkdir(parents=True, exist_ok=True)
    command = build_extract_audio_command(
        require_binary("ffmpeg"),
        input_path,
        audio_path,
        overwrite=overwrite,
        sample_rate=sample_rate,
        channels=channels,
    )
    try:
        completed = run_command(command, verbose=verbose, cancel_token=cancel_token)
    except OperationCancelled:
        audio_path.unlink(missing_ok=True)
        raise
    if completed.returncode != 0:
        raise command_error("ffmpeg", completed)
    if not audio_path.exists():
        raise VideoTranscribeError(f"ffmpeg completed but did not create audio output: {audio_path}")
    return audio_path


def build_whisper_command(
    whisper_bin: Path,
    model_path: Path,
    audio_path: Path,
    transcript_prefix: Path,
    *,
    language: str,
    threads: int | None,
    translate: bool,
    no_gpu: bool,
    no_timestamps: bool,
    print_progress: bool = True,
    fast: bool = False,
) -> list[str]:
    command = [
        str(whisper_bin),
        "-m",
        str(model_path),
        "-f",
        str(audio_path),
        "-l",
        language,
        "-otxt",
        "-of",
        str(transcript_prefix),
    ]
    if threads:
        command.extend(["-t", str(threads)])
    if translate:
        command.append("--translate")
    if no_gpu:
        command.append("--no-gpu")
    if no_timestamps:
        command.append("--no-timestamps")
    if print_progress:
        command.append("--print-progress")
    if fast:
        command.extend(["--best-of", "1", "--beam-size", "1", "--no-fallback"])
    return command


def transcribe_audio(
    audio_path: Path,
    transcript_path: Path,
    *,
    whisper_bin: Path,
    model_path: Path,
    language: str = DEFAULT_LANGUAGE,
    threads: int | None = None,
    translate: bool = False,
    no_gpu: bool = False,
    no_timestamps: bool = False,
    print_progress: bool = True,
    fast: bool = False,
    simplify_chinese: bool = DEFAULT_SIMPLIFY_CHINESE,
    cancel_token: CancellationToken | None = None,
    overwrite: bool = False,
    verbose: bool = False,
) -> Path:
    if not audio_path.exists():
        raise VideoTranscribeError(f"Audio file does not exist: {audio_path}")
    prepare_transcript_output(transcript_path, overwrite=overwrite)
    transcript_prefix = transcript_prefix_for(transcript_path)
    command = build_whisper_command(
        whisper_bin,
        model_path,
        audio_path,
        transcript_prefix,
        language=language,
        threads=threads,
        translate=translate,
        no_gpu=no_gpu,
        no_timestamps=no_timestamps,
        print_progress=print_progress,
        fast=fast,
    )
    run_kwargs: dict[str, Any] = {
        "verbose": verbose,
        "stream_output": print_progress,
    }
    if cancel_token is not None:
        run_kwargs["cancel_token"] = cancel_token
    try:
        completed = run_command(command, **run_kwargs)
    except OperationCancelled:
        transcript_path.unlink(missing_ok=True)
        raise
    if completed.returncode != 0:
        raise command_error("whisper.cpp", completed)
    if not transcript_path.exists():
        raise VideoTranscribeError(f"whisper.cpp completed but did not create transcript: {transcript_path}")
    if simplify_chinese:
        simplify_transcript_file(transcript_path)
    return transcript_path


def normalize_optional_model(value: str | None) -> str | None:
    if value is None:
        return None
    stripped = value.strip()
    if not stripped or stripped.lower() in {"none", "off", "false", "no", "disabled"}:
        return None
    return stripped


def extract_funasr_text(result: object) -> str:
    if isinstance(result, str):
        return result
    if isinstance(result, dict):
        text = result.get("text")
        return text if isinstance(text, str) else ""
    if isinstance(result, list):
        parts = [extract_funasr_text(item) for item in result]
        return "\n".join(part for part in parts if part)
    return ""


def strip_funasr_rich_markers(text: str) -> str:
    return FUNASR_RICH_MARKER_RE.sub("", text).strip()


def postprocess_funasr_text(text: str, *, rich_text: bool = DEFAULT_FUNASR_RICH_TEXT) -> str:
    try:
        from funasr.utils.postprocess_utils import rich_transcription_postprocess
    except ImportError:
        processed = text
    else:
        try:
            processed = rich_transcription_postprocess(text)
        except Exception:
            processed = text
    processed = FUNASR_RICH_TAG_RE.sub("", processed).strip()
    if not rich_text:
        processed = strip_funasr_rich_markers(processed)
    return processed


def convert_chinese_to_simplified(text: str) -> str:
    try:
        from opencc import OpenCC
    except ImportError as exc:
        raise VideoTranscribeError(
            "OpenCC is required for simplified Chinese transcript output. "
            "Install it with: python -m pip install opencc-python-reimplemented, "
            "or pass --no-simplify-chinese."
        ) from exc
    try:
        return OpenCC("t2s").convert(text)
    except Exception as exc:
        raise VideoTranscribeError(f"OpenCC traditional-to-simplified conversion failed: {exc}") from exc


def simplify_transcript_file(transcript_path: Path) -> Path:
    try:
        original = transcript_path.read_text(encoding="utf-8")
    except OSError as exc:
        raise VideoTranscribeError(f"Could not read transcript for OpenCC conversion: {transcript_path}") from exc
    simplified = convert_chinese_to_simplified(original)
    if simplified != original:
        try:
            transcript_path.write_text(simplified, encoding="utf-8")
        except OSError as exc:
            raise VideoTranscribeError(f"Could not write simplified transcript: {transcript_path}") from exc
    return transcript_path


def transcribe_audio_with_funasr(
    audio_path: Path,
    transcript_path: Path,
    *,
    model: str = DEFAULT_FUNASR_MODEL,
    device: str = DEFAULT_FUNASR_DEVICE,
    vad_model: str | None = DEFAULT_FUNASR_VAD_MODEL,
    punc_model: str | None = DEFAULT_FUNASR_PUNC_MODEL,
    batch_size_s: int = DEFAULT_FUNASR_BATCH_SIZE_S,
    rich_text: bool = DEFAULT_FUNASR_RICH_TEXT,
    simplify_chinese: bool = DEFAULT_SIMPLIFY_CHINESE,
    cancel_token: CancellationToken | None = None,
    overwrite: bool = False,
    verbose: bool = False,
) -> Path:
    if not audio_path.exists():
        raise VideoTranscribeError(f"Audio file does not exist: {audio_path}")
    if cancel_token is not None:
        cancel_token.raise_if_cancelled()
    prepare_transcript_output(transcript_path, overwrite=overwrite)

    try:
        from funasr import AutoModel
    except ImportError as exc:
        raise VideoTranscribeError(
            "FunASR is not installed in the current Python environment. "
            "Install it with: python -m pip install funasr modelscope torch"
        ) from exc

    model_kwargs: dict[str, object] = {
        "model": resolve_managed_funasr_model(model),
        "device": device,
        "disable_update": True,
        # Local paths still trigger ModelScope's latest-version HTTP check
        # unless this is disabled explicitly. Runtime network is intentionally
        # unavailable for this skill after installation.
        "check_latest": False,
    }
    normalized_vad_model = normalize_optional_model(vad_model)
    normalized_punc_model = normalize_optional_model(punc_model)
    if normalized_vad_model:
        model_kwargs["vad_model"] = resolve_managed_funasr_model(normalized_vad_model)
        model_kwargs["vad_kwargs"] = {"check_latest": False}
    if normalized_punc_model:
        model_kwargs["punc_model"] = normalized_punc_model
    if verbose:
        print(f"funasr_model: {model_kwargs}", file=sys.stderr)

    try:
        recognizer = AutoModel(**model_kwargs)
        result = recognizer.generate(
            input=str(audio_path),
            batch_size_s=batch_size_s,
            use_itn=True,
        )
        if cancel_token is not None:
            cancel_token.raise_if_cancelled()
    except OperationCancelled:
        raise
    except Exception as exc:
        raise VideoTranscribeError(f"FunASR failed: {exc}") from exc

    text = postprocess_funasr_text(extract_funasr_text(result), rich_text=rich_text)
    if not text:
        raise VideoTranscribeError("FunASR completed but returned no transcript text")
    if simplify_chinese:
        text = convert_chinese_to_simplified(text)
    transcript_path.write_text(text + "\n", encoding="utf-8")
    return transcript_path


def resolve_managed_funasr_model(model: str) -> str:
    if Path(model).is_dir():
        return model
    relative = FUNASR_MANAGED_MODEL_PATHS.get(model)
    cache = os.environ.get("MODELSCOPE_CACHE", "").strip()
    if relative is None or not cache:
        return model
    candidate = Path(cache).joinpath(*relative)
    if candidate.is_dir() and (candidate / "model.pt").is_file() and (
        (candidate / "config.yaml").is_file()
        or (candidate / "configuration.json").is_file()
    ):
        return str(candidate)
    raise VideoTranscribeError(
        f"Installed FunASR model is missing or incomplete: {candidate}. "
        "Repair or reinstall the media skill before transcription."
    )


def transcribe_audio_with_engine(
    audio_path: Path,
    transcript_path: Path,
    *,
    engine: str = DEFAULT_TRANSCRIBE_ENGINE,
    whisper_bin: Path | None = None,
    whisper_model_path: Path | None = None,
    language: str = DEFAULT_LANGUAGE,
    threads: int | None = None,
    translate: bool = False,
    no_gpu: bool = False,
    no_timestamps: bool = False,
    print_progress: bool = True,
    fast: bool = False,
    funasr_model: str = DEFAULT_FUNASR_MODEL,
    funasr_device: str = DEFAULT_FUNASR_DEVICE,
    funasr_vad_model: str | None = DEFAULT_FUNASR_VAD_MODEL,
    funasr_punc_model: str | None = DEFAULT_FUNASR_PUNC_MODEL,
    funasr_batch_size_s: int = DEFAULT_FUNASR_BATCH_SIZE_S,
    funasr_rich_text: bool = DEFAULT_FUNASR_RICH_TEXT,
    simplify_chinese: bool = DEFAULT_SIMPLIFY_CHINESE,
    cancel_token: CancellationToken | None = None,
    overwrite: bool = False,
    verbose: bool = False,
) -> Path:
    normalized_engine = engine.lower()
    if normalized_engine == "whisper":
        if whisper_bin is None:
            raise VideoTranscribeError("whisper.cpp binary was not resolved")
        if whisper_model_path is None:
            raise VideoTranscribeError("whisper.cpp model was not resolved")
        return transcribe_audio(
            audio_path,
            transcript_path,
            whisper_bin=whisper_bin,
            model_path=whisper_model_path,
            language=language,
            threads=threads,
            translate=translate,
            no_gpu=no_gpu,
            no_timestamps=no_timestamps,
            print_progress=print_progress,
            fast=fast,
            simplify_chinese=simplify_chinese,
            cancel_token=cancel_token,
            overwrite=overwrite,
            verbose=verbose,
        )
    if normalized_engine == "funasr":
        return transcribe_audio_with_funasr(
            audio_path,
            transcript_path,
            model=funasr_model,
            device=funasr_device,
            vad_model=funasr_vad_model,
            punc_model=funasr_punc_model,
            batch_size_s=funasr_batch_size_s,
            rich_text=funasr_rich_text,
            simplify_chinese=simplify_chinese,
            cancel_token=cancel_token,
            overwrite=overwrite,
            verbose=verbose,
        )
    raise VideoTranscribeError(f"Unsupported transcription engine: {engine}")


def should_transcribe_input_directly(input_path: Path, audio_output: str | None) -> bool:
    return input_path.suffix.lower() == ".wav" and audio_output is None


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract audio from a media file and transcribe it with a local ASR engine.",
    )
    parser.add_argument("input", nargs="?", help="Input video/audio file. Defaults to latest media in downloads/.")
    parser.add_argument("--downloads-dir", default=DEFAULT_DOWNLOAD_DIR, help="Directory used when input is omitted. Default: downloads")
    parser.add_argument("--audio-output", help="Output WAV path. Default: input stem plus _audio.wav")
    parser.add_argument("--text-output", help="Output transcript TXT path. Default: input stem plus _transcript.txt")
    parser.add_argument("--output-dir", help="Directory for default audio/transcript outputs. Default: input file directory")
    parser.add_argument("--engine", choices=TRANSCRIBE_ENGINES, default=DEFAULT_TRANSCRIBE_ENGINE, help=f"Transcription engine. Default: {DEFAULT_TRANSCRIBE_ENGINE}")
    parser.add_argument("--whisper-bin", help="Path or executable name for whisper.cpp whisper-cli.")
    parser.add_argument("--model", help="Path to a whisper.cpp ggml model.")
    parser.add_argument("--language", default=DEFAULT_LANGUAGE, help="Spoken language, or auto. Default: auto")
    parser.add_argument("--threads", type=int, default=default_whisper_threads(), help=f"Thread count passed to whisper.cpp. Default: auto, capped at {DEFAULT_MAX_THREADS}")
    parser.add_argument("--sample-rate", type=int, default=DEFAULT_SAMPLE_RATE, help="Audio sample rate for extracted WAV. Default: 16000")
    parser.add_argument("--channels", type=int, default=DEFAULT_CHANNELS, help="Audio channel count for extracted WAV. Default: 1")
    parser.add_argument("--translate", action="store_true", help="Ask whisper.cpp to translate speech to English.")
    parser.add_argument("--fast", action="store_true", help="Use faster greedy whisper.cpp decoding. May reduce transcription quality.")
    parser.add_argument("--no-gpu", action="store_true", help="Pass --no-gpu to whisper.cpp.")
    parser.add_argument("--timestamps", action="store_true", help="Keep timestamps in whisper.cpp text output.")
    parser.add_argument("--no-progress", dest="progress", action="store_false", default=True, help="Disable whisper.cpp progress output.")
    simplify_group = parser.add_mutually_exclusive_group()
    simplify_group.add_argument(
        "--simplify-chinese",
        dest="simplify_chinese",
        action="store_true",
        default=DEFAULT_SIMPLIFY_CHINESE,
        help="Convert transcript text from traditional to simplified Chinese with OpenCC. Default: enabled",
    )
    simplify_group.add_argument(
        "--no-simplify-chinese",
        dest="simplify_chinese",
        action="store_false",
        help="Keep the ASR engine's original Chinese script without OpenCC conversion.",
    )
    parser.add_argument("--extract-only", action="store_true", help="Only extract audio; do not run STT.")
    parser.add_argument("--reuse-audio", action="store_true", help="Reuse existing audio output instead of extracting again.")
    parser.add_argument("--overwrite", action="store_true", help="Overwrite existing audio/transcript outputs.")
    parser.add_argument("--funasr-model", default=DEFAULT_FUNASR_MODEL, help=f"FunASR model id or local path. Default: {DEFAULT_FUNASR_MODEL}")
    parser.add_argument("--funasr-device", default=DEFAULT_FUNASR_DEVICE, help=f"FunASR device. Default: {DEFAULT_FUNASR_DEVICE}")
    parser.add_argument("--funasr-vad-model", default=DEFAULT_FUNASR_VAD_MODEL, help=f"FunASR VAD model, or none/off. Default: {DEFAULT_FUNASR_VAD_MODEL}")
    parser.add_argument("--funasr-punc-model", default=DEFAULT_FUNASR_PUNC_MODEL, help="Optional FunASR punctuation model, or none/off. Default: none")
    parser.add_argument("--funasr-batch-size-s", type=int, default=DEFAULT_FUNASR_BATCH_SIZE_S, help=f"FunASR batch duration in seconds. Default: {DEFAULT_FUNASR_BATCH_SIZE_S}")
    parser.add_argument("--funasr-rich-text", action="store_true", default=DEFAULT_FUNASR_RICH_TEXT, help="Keep SenseVoice rich transcription emoji for emotion and audio events. Default: off")
    parser.add_argument("-v", "--verbose", action="store_true", help="Print ffmpeg command and ASR command/config details.")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        input_path = Path(args.input).expanduser() if args.input else latest_media(Path(args.downloads_dir).expanduser())
        audio_path = output_path_for(input_path, args.audio_output, args.output_dir, DEFAULT_AUDIO_SUFFIX, ".wav")
        transcript_path = transcript_output_path_for(
            input_path,
            args.text_output,
            args.output_dir,
        )
        if should_transcribe_input_directly(input_path, args.audio_output):
            if not input_path.exists():
                raise VideoTranscribeError(f"Input file does not exist: {input_path}")
            saved_audio = input_path
        else:
            progress_total = 2 if args.extract_only else 3
            emit_parent_progress(
                "media_download.transcribe.extracting_audio",
                1,
                progress_total,
            )
            auto_reuse_audio = not args.extract_only and args.audio_output is None and audio_path.exists()
            saved_audio = extract_audio(
                input_path,
                audio_path,
                overwrite=args.overwrite,
                reuse_audio=args.reuse_audio or auto_reuse_audio,
                sample_rate=args.sample_rate,
                channels=args.channels,
                verbose=args.verbose,
            )
        print(f"audio: {saved_audio}")
        if args.extract_only:
            return 0

        emit_parent_progress(
            "media_download.transcribe.recognizing_speech",
            2,
            3,
        )
        whisper_bin = find_whisper_binary(args.whisper_bin) if args.engine == "whisper" else None
        model_path = find_whisper_model(args.model) if args.engine == "whisper" else None
        transcript = transcribe_audio_with_engine(
            saved_audio,
            transcript_path,
            engine=args.engine,
            whisper_bin=whisper_bin,
            whisper_model_path=model_path,
            language=args.language,
            threads=args.threads,
            translate=args.translate,
            no_gpu=args.no_gpu,
            no_timestamps=not args.timestamps,
            print_progress=args.progress,
            fast=args.fast,
            funasr_model=args.funasr_model,
            funasr_device=args.funasr_device,
            funasr_vad_model=args.funasr_vad_model,
            funasr_punc_model=args.funasr_punc_model,
            funasr_batch_size_s=args.funasr_batch_size_s,
            funasr_rich_text=args.funasr_rich_text,
            simplify_chinese=args.simplify_chinese,
            overwrite=args.overwrite,
            verbose=args.verbose,
        )
        print(f"transcript: {transcript}")
        return 0
    except (VideoTranscribeError, OSError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
