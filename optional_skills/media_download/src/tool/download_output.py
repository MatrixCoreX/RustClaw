"""Explicit download output selection, independent from transcription inputs."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import os


class OutputSelectionError(RuntimeError):
    def __init__(self, message, error_code):
        super().__init__(message)
        self.error_code = error_code


def validate_output_mode(mode):
    if mode is not None and (not isinstance(mode, str) or mode not in {"video", "audio", "video_audio"}):
        raise OutputSelectionError("output_mode must be video, audio, or video_audio", "invalid_output_mode")


def select_download_output(artifacts, mode, artifact_factory, run=None):
    """Omitted mode preserves existing delivery, including image/article posts."""
    validate_output_mode(mode)
    if mode is None:
        return artifacts
    videos = [a for a in artifacts if a.get("mime_type", "").startswith("video/")]
    audio = [a for a in artifacts if a.get("mime_type", "").startswith("audio/")]
    if mode == "video":
        if not videos:
            raise OutputSelectionError("No video was available for this post", "video_not_found")
        return videos
    if not audio and videos:
        ffmpeg = shutil.which("ffmpeg")
        if not ffmpeg:
            raise OutputSelectionError("Audio extraction requires ffmpeg", "ffmpeg_unavailable")
        source = Path(videos[0]["path"])
        fd, filename = tempfile.mkstemp(prefix=source.stem + "_audio_", suffix=".m4a", dir=source.parent)
        os.close(fd)
        target = Path(filename)
        command = [ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-protocol_whitelist", "file,pipe", "-i", str(source),
                   "-map", "0:a:0", "-vn", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", str(target)]
        try:
            completed = (run or subprocess.run)(command, capture_output=True, text=True, timeout=300)
        except (OSError, subprocess.TimeoutExpired):
            target.unlink(missing_ok=True)
            raise OutputSelectionError("Audio extraction did not complete", "audio_extraction_failed")
        if completed.returncode == 0 and target.is_file() and target.stat().st_size:
            item = artifact_factory(target)
            item["mime_type"] = "audio/mp4"
            item["artifact_role"] = "extracted_audio"
            audio = [item]
        else:
            target.unlink(missing_ok=True)
    # Keep a successfully downloaded video when its source has no audio track.
    # The caller's required-kind contract marks this as partial delivery.
    selected = audio if mode == "audio" else videos + audio
    if not selected:
        raise OutputSelectionError("No audio track was available for this post", "audio_not_found")
    return selected
