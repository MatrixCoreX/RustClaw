"""Lightweight audio helpers for media inspection and component extraction."""
from __future__ import annotations

from pathlib import Path
import shutil
import subprocess


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
    """Return whether ffprobe finds an audio stream, or None when unavailable."""
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
