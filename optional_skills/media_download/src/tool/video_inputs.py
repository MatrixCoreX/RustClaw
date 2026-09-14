"""Prepare local video inputs without invoking any speech-recognition engine."""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile
from typing import Any, Callable
import wave

from video_transcriber import build_extract_audio_command


def prepare_video_inputs(
    artifacts: list[dict[str, Any]],
    existing: dict[str, Any] | None,
    output_dir: Path,
    scope: str,
    describe: Callable[[Path], dict[str, Any]],
    progress: Any = None,
) -> dict[str, Any] | None:
    if scope not in {"images_only", "audio_only", "images_and_audio"}:
        return existing
    video = next((item for item in artifacts if item.get("artifact_role") == "original_video"), None)
    if not video or not video.get("path"):
        return existing
    source = Path(str(video["path"]))
    inputs = dict(existing or {})
    components = []
    if scope in {"images_only", "images_and_audio"}:
        components.append(("video_first_frame", ".png"))
    if scope in {"audio_only", "images_and_audio"}:
        components.append(("video_audio", ".wav"))

    for component, suffix in components:
        output = None
        is_audio = component == "video_audio"
        if is_audio and progress is not None:
            progress.emit("media_download.extract_audio.starting", current=0, total=1)
        try:
            # Allocate our own file; never overwrite a prior result or source.
            fd, name = tempfile.mkstemp(prefix=f"{source.stem}_{component}_", suffix=suffix, dir=output_dir)
            os.close(fd)
            output = Path(name)
            if is_audio:
                command = build_extract_audio_command(
                    "ffmpeg", source, output, overwrite=True, sample_rate=16000, channels=1,
                )
                command[1:1] = ["-nostdin", "-loglevel", "error", "-protocol_whitelist", "file,pipe"]
            else:
                command = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin",
                           "-protocol_whitelist", "file,pipe", "-i", str(source),
                           "-frames:v", "1", "-y", str(output)]
            completed = subprocess.run(command, capture_output=True, text=True, check=False)
            if completed.returncode != 0 or output.stat().st_size == 0:
                raise ValueError("ffmpeg did not produce the requested component")
            descriptor = describe(output)
            if is_audio:
                with wave.open(str(output), "rb") as audio:
                    if audio.getnframes() == 0:
                        raise ValueError("audio stream is empty")
                    descriptor.update({"sample_rate": audio.getframerate(), "channels": audio.getnchannels()})
                descriptor["engine"] = "ffmpeg"
            descriptor.update({"status": "available", "source": component, "deliver_to_user": False})
            inputs[component] = descriptor
            if is_audio and progress is not None:
                progress.emit("media_download.extract_audio.completed", current=1, total=1)
        except (OSError, ValueError, EOFError, wave.Error):
            if output is not None:
                try:
                    output.unlink(missing_ok=True)
                except OSError:
                    pass  # Cleanup failure must not hide the original download.
            inputs[component] = {"status": "unavailable", "error_code": f"{component}_extraction_failed",
                                 "message_key": f"media_download.error.{component}_extraction_failed",
                                 "retryable": False}
    return inputs
