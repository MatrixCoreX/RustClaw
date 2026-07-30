#!/usr/bin/env python3
"""
Transcode videos to an X-compatible MP4 profile.

Default output:
  - MP4 container
  - H.264 video, yuv420p, 30 fps
  - AAC-LC audio
  - faststart metadata for web upload/processing
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from types import SimpleNamespace
from pathlib import Path
from typing import Any


DEFAULT_DOWNLOAD_DIR = "downloads"
DEFAULT_SUFFIX = "_x"
OUTPUT_TIME_FORMAT = "%Y%m%d_%H%M%S"
VIDEO_EXTENSIONS = {".mp4", ".mov", ".mkv", ".webm", ".avi", ".flv", ".m4v", ".ts"}


class TranscodeError(RuntimeError):
    """Raised when probing or transcoding cannot be completed."""


@dataclass(frozen=True)
class MediaInfo:
    path: Path
    container: str | None
    size: int
    duration: float | None
    video_codec: str | None
    video_profile: str | None
    width: int | None
    height: int | None
    fps: float | None
    pixel_format: str | None
    video_bit_rate: int | None
    audio_codec: str | None
    audio_bit_rate: int | None


@dataclass(frozen=True)
class CompatibilityResult:
    ok: bool
    reasons: list[str]


@dataclass(frozen=True)
class BatchSummary:
    total: int
    compatible: int
    converted: int
    existing: int
    incompatible: int
    failed: int


def require_binary(name: str) -> str:
    binary = shutil.which(name)
    if not binary:
        raise TranscodeError(f"{name} is required but was not found in PATH.")
    return binary


def parse_number(value: Any) -> float | None:
    if value is None or value == "N/A":
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def parse_int(value: Any) -> int | None:
    number = parse_number(value)
    return int(number) if number is not None else None


def parse_fps(value: str | None) -> float | None:
    if not value or value == "0/0":
        return None
    if "/" not in value:
        return parse_number(value)
    numerator, denominator = value.split("/", 1)
    try:
        den = float(denominator)
        if den == 0:
            return None
        return float(numerator) / den
    except ValueError:
        return None


def ffprobe_json(path: Path) -> dict[str, Any]:
    ffprobe = require_binary("ffprobe")
    command = [
        ffprobe,
        "-v",
        "error",
        "-show_entries",
        (
            "format=format_name,duration,size,bit_rate:"
            "stream=index,codec_type,codec_name,profile,width,height,pix_fmt,"
            "r_frame_rate,avg_frame_rate,duration,bit_rate"
        ),
        "-of",
        "json",
        str(path),
    ]
    completed = subprocess.run(command, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        raise TranscodeError(completed.stderr.strip() or f"ffprobe failed for {path}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise TranscodeError(f"ffprobe returned invalid JSON for {path}") from exc


def probe_media(path: Path) -> MediaInfo:
    if not path.exists():
        raise TranscodeError(f"Input file does not exist: {path}")
    payload = ffprobe_json(path)
    fmt = payload.get("format") or {}
    streams = payload.get("streams") or []
    video = next((stream for stream in streams if stream.get("codec_type") == "video"), {})
    audio = next((stream for stream in streams if stream.get("codec_type") == "audio"), {})
    fps = parse_fps(video.get("avg_frame_rate")) or parse_fps(video.get("r_frame_rate"))

    return MediaInfo(
        path=path,
        container=fmt.get("format_name"),
        size=parse_int(fmt.get("size")) or path.stat().st_size,
        duration=parse_number(fmt.get("duration")),
        video_codec=video.get("codec_name"),
        video_profile=video.get("profile"),
        width=parse_int(video.get("width")),
        height=parse_int(video.get("height")),
        fps=fps,
        pixel_format=video.get("pix_fmt"),
        video_bit_rate=parse_int(video.get("bit_rate")),
        audio_codec=audio.get("codec_name"),
        audio_bit_rate=parse_int(audio.get("bit_rate")),
    )


def latest_video(directory: Path) -> Path:
    if not directory.exists():
        raise TranscodeError(f"Directory does not exist: {directory}")
    candidates = [
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix.lower() in {".mp4", ".mov"}
    ]
    if not candidates:
        raise TranscodeError(f"No .mp4 or .mov files found in {directory}")
    return max(candidates, key=lambda path: path.stat().st_mtime)


def find_videos(
    directory: Path,
    *,
    recursive: bool = True,
    exclude_suffix: str | None = DEFAULT_SUFFIX,
) -> list[Path]:
    if not directory.exists():
        raise TranscodeError(f"Directory does not exist: {directory}")
    if not directory.is_dir():
        raise TranscodeError(f"Input is not a directory: {directory}")
    paths = directory.rglob("*") if recursive else directory.iterdir()
    return sorted(
        path
        for path in paths
        if path.is_file()
        and path.suffix.lower() in VIDEO_EXTENSIONS
        and (not exclude_suffix or not path.stem.endswith(exclude_suffix))
    )


def timestamp_output_name(suffix: str = "") -> str:
    return f"{time.strftime(OUTPUT_TIME_FORMAT)}{suffix}.mp4"


def output_path_for(
    input_path: Path,
    output: str | None,
    output_dir: str | None,
    suffix: str,
    *,
    use_time_name: bool = False,
) -> Path:
    if output:
        path = Path(output).expanduser()
        if path.suffix.lower() != ".mp4":
            path = path.with_suffix(".mp4")
        return path

    parent = Path(output_dir).expanduser() if output_dir else input_path.parent
    if use_time_name:
        return parent / timestamp_output_name(suffix)
    return parent / f"{input_path.stem}{suffix}.mp4"


def batch_output_path(
    input_path: Path,
    input_directory: Path,
    output_dir: str | None,
    suffix: str,
) -> Path:
    if not output_dir:
        return output_path_for(input_path, None, None, suffix)
    relative_parent = input_path.parent.relative_to(input_directory)
    return Path(output_dir).expanduser() / relative_parent / f"{input_path.stem}{suffix}.mp4"


def even_floor(value: int) -> int:
    return max(2, value - value % 2)


def target_dimensions(info: MediaInfo, max_landscape: tuple[int, int], max_portrait: tuple[int, int]) -> tuple[int, int] | None:
    if not info.width or not info.height:
        return None

    if info.width >= info.height:
        max_width, max_height = max_landscape
    else:
        max_width, max_height = max_portrait

    scale = min(max_width / info.width, max_height / info.height, 1.0)
    width = even_floor(round(info.width * scale))
    height = even_floor(round(info.height * scale))
    return width, height


def check_x_compatibility(
    info: MediaInfo,
    *,
    max_file_size: int,
    max_duration: float,
    max_fps: float,
    max_landscape: tuple[int, int],
    max_portrait: tuple[int, int],
) -> CompatibilityResult:
    reasons: list[str] = []
    suffix = info.path.suffix.lower()
    if suffix not in {".mp4", ".mov"}:
        reasons.append("container extension is not .mp4 or .mov")
    if "mp4" not in str(info.container).lower() and "mov" not in str(info.container).lower():
        reasons.append(f"container is {info.container or 'unknown'}, not MP4/MOV")
    if info.video_codec != "h264":
        reasons.append(f"video codec is {info.video_codec or 'missing'}, expected h264")
    if info.audio_codec and info.audio_codec != "aac":
        reasons.append(f"audio codec is {info.audio_codec}, expected aac")
    if info.pixel_format and info.pixel_format != "yuv420p":
        reasons.append(f"pixel format is {info.pixel_format}, expected yuv420p")
    if info.fps and info.fps > max_fps:
        reasons.append(f"frame rate is {info.fps:.2f}, expected <= {max_fps:g}")
    if info.size > max_file_size:
        reasons.append(f"file size is {info.size} bytes, expected <= {max_file_size}")
    if info.duration and info.duration > max_duration:
        reasons.append(f"duration is {info.duration:.2f}s, expected <= {max_duration:g}s")
    if info.width and info.height:
        target = target_dimensions(info, max_landscape, max_portrait)
        if target and target != (info.width, info.height):
            reasons.append(f"resolution is {info.width}x{info.height}, target is {target[0]}x{target[1]}")
    else:
        reasons.append("video dimensions are missing")
    return CompatibilityResult(ok=not reasons, reasons=reasons)


def default_options(**overrides: Any) -> SimpleNamespace:
    values: dict[str, Any] = {
        "overwrite": False,
        "preset": "medium",
        "crf": 23,
        "fps": 30,
        "audio_bitrate": "128k",
        "h264_profile": "high",
        "h264_level": "4.1",
        "max_file_size_mb": 512,
        "max_duration": 140.0,
        "max_fps": 40.0,
        "max_landscape_width": 1920,
        "max_landscape_height": 1080,
        "max_portrait_width": 1080,
        "max_portrait_height": 1920,
        "verbose": False,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def check_with_options(info: MediaInfo, options: argparse.Namespace | SimpleNamespace) -> CompatibilityResult:
    return check_x_compatibility(
        info,
        max_file_size=options.max_file_size_mb * 1024 * 1024,
        max_duration=options.max_duration,
        max_fps=options.max_fps,
        max_landscape=(options.max_landscape_width, options.max_landscape_height),
        max_portrait=(options.max_portrait_width, options.max_portrait_height),
    )


def print_media_summary(info: MediaInfo, compatibility: CompatibilityResult | None = None) -> None:
    print(f"file: {info.path}")
    print(f"size: {info.size / 1024 / 1024:.2f} MiB ({info.size} bytes)")
    if info.duration is not None:
        print(f"duration: {info.duration:.2f}s")
    if info.width and info.height:
        print(f"resolution: {info.width}x{info.height}")
    if info.fps:
        print(f"fps: {info.fps:.2f}")
    print(f"video: {info.video_codec or 'missing'}" + (f" ({info.video_profile})" if info.video_profile else ""))
    print(f"pixel_format: {info.pixel_format or 'unknown'}")
    print(f"audio: {info.audio_codec or 'none'}")
    if compatibility:
        print("x_compatible: yes" if compatibility.ok else "x_compatible: no")
        for reason in compatibility.reasons:
            print(f"- {reason}")


def build_ffmpeg_command(args: argparse.Namespace, input_path: Path, output_path: Path, info: MediaInfo) -> list[str]:
    ffmpeg = require_binary("ffmpeg")
    target = target_dimensions(info, (args.max_landscape_width, args.max_landscape_height), (args.max_portrait_width, args.max_portrait_height))
    filters: list[str] = []
    if target and target != (info.width, info.height):
        filters.append(f"scale={target[0]}:{target[1]}:flags=lanczos")

    command = [
        ffmpeg,
        "-hide_banner",
        # X transcoding can run on the interactive queue's worker thread.
        # Do not let ffmpeg consume keystrokes intended for the main prompt.
        "-nostdin",
        "-y" if args.overwrite else "-n",
        "-i",
        str(input_path),
        "-map",
        "0:v:0",
        "-map",
        "0:a?",
        "-c:v",
        "libx264",
        "-preset",
        args.preset,
        "-crf",
        str(args.crf),
        "-pix_fmt",
        "yuv420p",
        "-r",
        str(args.fps),
        "-profile:v",
        args.h264_profile,
        "-level",
        args.h264_level,
        "-tag:v",
        "avc1",
    ]
    if filters:
        command.extend(["-vf", ",".join(filters)])
    command.extend(
        [
            "-c:a",
            "aac",
            "-b:a",
            args.audio_bitrate,
            "-ac",
            "2",
            "-ar",
            "44100",
            "-movflags",
            "+faststart",
            str(output_path),
        ]
    )
    return command


def transcode(args: argparse.Namespace, input_path: Path, output_path: Path, info: MediaInfo) -> Path:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    if output_path.exists() and not args.overwrite:
        raise TranscodeError(f"Output already exists, pass --overwrite to replace it: {output_path}")

    command = build_ffmpeg_command(args, input_path, output_path, info)
    if args.verbose:
        print(" ".join(command), file=sys.stderr)
    runner = getattr(args, "subprocess_runner", subprocess.run)
    completed = runner(command, check=False)
    if completed.returncode != 0:
        raise TranscodeError(f"ffmpeg failed with exit code {completed.returncode}")
    return output_path


def process_directory(args: argparse.Namespace | SimpleNamespace, directory: Path) -> BatchSummary:
    if getattr(args, "output", None):
        raise TranscodeError("--output cannot be used when the input is a directory; use --output-dir.")
    check_only = bool(getattr(args, "check", False))
    suffix = str(getattr(args, "suffix", DEFAULT_SUFFIX))
    recursive = bool(getattr(args, "recursive", True))
    files = find_videos(
        directory,
        recursive=recursive,
        exclude_suffix=None if check_only else suffix,
    )
    if not files:
        raise TranscodeError(f"No supported video files found in {directory}")

    compatible_count = 0
    converted_count = 0
    existing_count = 0
    incompatible_count = 0
    failed_count = 0
    print(
        f"x_batch_start: folder={directory} videos={len(files)} recursive={'yes' if recursive else 'no'}"
    )
    for index, input_path in enumerate(files, start=1):
        print(f"x_batch_item: {index}/{len(files)} file={input_path}")
        try:
            info = probe_media(input_path)
            compatibility = check_with_options(info, args)
            if compatibility.ok:
                compatible_count += 1
                print("x_compatible: yes")
            else:
                incompatible_count += 1
                print("x_compatible: no")
                for reason in compatibility.reasons:
                    print(f"- {reason}")

            if check_only:
                continue
            # Folder mode is incremental: compatible originals are never duplicated,
            # even when --force was configured for single-file conversions.
            if compatibility.ok:
                print("x_batch_skipped: already_compatible")
                continue

            output_path = batch_output_path(
                input_path,
                directory,
                getattr(args, "output_dir", None),
                suffix,
            )
            if output_path.exists() and not bool(getattr(args, "overwrite", False)):
                existing_info = probe_media(output_path)
                existing_compatibility = check_with_options(existing_info, args)
                if not existing_compatibility.ok:
                    raise TranscodeError(
                        f"Output exists but is not X-compatible; pass --overwrite: {output_path}"
                    )
                existing_count += 1
                print(f"x_batch_existing: {output_path}")
                continue

            converted = transcode(args, input_path, output_path, info)
            converted_info = probe_media(converted)
            converted_compatibility = check_with_options(converted_info, args)
            if not converted_compatibility.ok:
                reasons = "; ".join(converted_compatibility.reasons)
                raise TranscodeError(f"Converted output is not X-compatible: {reasons}")
            converted_count += 1
            print(f"x_batch_converted: {converted}")
        except (OSError, TranscodeError) as exc:
            failed_count += 1
            print(f"x_batch_failed: {input_path}: {exc}", file=sys.stderr)

    summary = BatchSummary(
        total=len(files),
        compatible=compatible_count,
        converted=converted_count,
        existing=existing_count,
        incompatible=incompatible_count,
        failed=failed_count,
    )
    print(
        "x_batch_completed: "
        f"total={summary.total} compatible={summary.compatible} "
        f"incompatible={summary.incompatible} converted={summary.converted} "
        f"existing={summary.existing} failed={summary.failed}"
    )
    return summary


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Transcode a video to an X-compatible MP4.")
    parser.add_argument(
        "input",
        nargs="?",
        help="Input video or directory. A directory is scanned recursively by default.",
    )
    parser.add_argument("-o", "--output", help="Output file path. Default: current local time plus suffix, e.g. 20260624_153012_x.mp4")
    parser.add_argument("--output-dir", help="Output directory. Ignored when --output is set.")
    parser.add_argument("--downloads-dir", default=DEFAULT_DOWNLOAD_DIR, help="Directory used when input is omitted. Default: downloads")
    parser.add_argument("--suffix", default=DEFAULT_SUFFIX, help="Suffix for default output filename. Default: _x")
    parser.add_argument("--check", action="store_true", help="Only check compatibility; do not transcode.")
    parser.add_argument(
        "--force",
        action="store_true",
        help="Transcode an already compatible single-file input; directory mode always skips it.",
    )
    recursive_group = parser.add_mutually_exclusive_group()
    recursive_group.add_argument(
        "--recursive",
        dest="recursive",
        action="store_true",
        default=True,
        help="Recursively scan a directory input. Default: enabled",
    )
    recursive_group.add_argument(
        "--no-recursive",
        dest="recursive",
        action="store_false",
        help="Only scan video files directly inside the input directory.",
    )
    parser.add_argument("--overwrite", action="store_true", help="Overwrite output if it already exists.")
    parser.add_argument("--crf", type=int, default=23, help="x264 CRF quality. Lower is larger/better. Default: 23")
    parser.add_argument("--preset", default="medium", help="x264 preset. Default: medium")
    parser.add_argument("--fps", type=int, default=30, help="Output frame rate. Default: 30")
    parser.add_argument("--audio-bitrate", default="128k", help="AAC audio bitrate. Default: 128k")
    parser.add_argument("--h264-profile", default="high", choices=("baseline", "main", "high"), help="H.264 profile. Default: high")
    parser.add_argument("--h264-level", default="4.1", help="H.264 level. Default: 4.1")
    parser.add_argument("--max-file-size-mb", type=int, default=512, help="Compatibility check file size limit. Default: 512")
    parser.add_argument("--max-duration", type=float, default=140.0, help="Compatibility check duration limit in seconds. Default: 140")
    parser.add_argument("--max-fps", type=float, default=40.0, help="Compatibility check frame-rate limit. Default: 40")
    parser.add_argument("--max-landscape-width", type=int, default=1920, help="Max landscape output width. Default: 1920")
    parser.add_argument("--max-landscape-height", type=int, default=1080, help="Max landscape output height. Default: 1080")
    parser.add_argument("--max-portrait-width", type=int, default=1080, help="Max portrait output width. Default: 1080")
    parser.add_argument("--max-portrait-height", type=int, default=1920, help="Max portrait output height. Default: 1920")
    parser.add_argument("-v", "--verbose", action="store_true", help="Print ffmpeg command.")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        input_path = Path(args.input).expanduser() if args.input else latest_video(Path(args.downloads_dir).expanduser())
        if input_path.is_dir():
            summary = process_directory(args, input_path)
            if summary.failed:
                return 1
            if args.check and summary.incompatible:
                return 2
            return 0

        info = probe_media(input_path)
        compatibility = check_with_options(info, args)
        print_media_summary(info, compatibility)

        if args.check:
            return 0 if compatibility.ok else 2
        if compatibility.ok and not args.force:
            print("Already compatible; no transcoding needed.")
            return 0

        output_path = output_path_for(input_path, args.output, args.output_dir, args.suffix, use_time_name=True)
        saved_path = transcode(args, input_path, output_path, info)
        print(f"output: {saved_path}")
        output_info = probe_media(saved_path)
        output_compatibility = check_with_options(output_info, args)
        print_media_summary(output_info, output_compatibility)
        return 0 if output_compatibility.ok else 2
    except TranscodeError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
