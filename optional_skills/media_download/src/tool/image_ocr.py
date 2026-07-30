#!/usr/bin/env python3
"""
Run OCR on local image files with the Tesseract command-line engine.
"""

from __future__ import annotations

import argparse
import csv
from contextlib import ExitStack
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path


DEFAULT_DOWNLOAD_DIR = "downloads"
DEFAULT_LANGUAGE = "chi_sim"
DEFAULT_PSM = 6
DEFAULT_OEM = 1
DEFAULT_PREPROCESS = True
DEFAULT_MIN_LINE_CONFIDENCE = 15.0
DEFAULT_SUFFIX = "_ocr"
PREPROCESS_SCALE = 2
PREPROCESS_CONTRAST = 2.0
PREPROCESS_THRESHOLD = 180
IMAGE_EXTENSIONS = {
    ".jpg",
    ".jpeg",
    ".png",
    ".webp",
    ".tif",
    ".tiff",
    ".bmp",
    ".gif",
    ".avif",
}


class ImageOcrError(RuntimeError):
    """Raised when image OCR cannot be completed."""


@dataclass(frozen=True)
class OcrResult:
    path: Path
    text: str


@dataclass(frozen=True)
class ParsedOcrText:
    text: str
    confidence: float


def require_binary(name: str) -> str:
    binary = shutil.which(name)
    if not binary:
        raise ImageOcrError(f"{name} is required but was not found in PATH.")
    return binary


def latest_image(directory: Path) -> Path:
    if not directory.exists():
        raise ImageOcrError(f"Directory does not exist: {directory}")
    candidates = [
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix.lower() in IMAGE_EXTENSIONS
    ]
    if not candidates:
        raise ImageOcrError(f"No image files found in {directory}")
    return max(candidates, key=lambda path: path.stat().st_mtime)


def normalize_ocr_text(text: str) -> str:
    text = text.replace("\f", "")
    lines = [line.rstrip() for line in text.splitlines()]
    while lines and not lines[0].strip():
        lines.pop(0)
    while lines and not lines[-1].strip():
        lines.pop()
    return "\n".join(lines)


def build_tesseract_command(
    tesseract_bin: Path | str,
    image_path: Path,
    *,
    language: str,
    psm: int | None = None,
    oem: int | None = None,
    configs: dict[str, str] | None = None,
    output_format: str | None = None,
) -> list[str]:
    command = [str(tesseract_bin), str(image_path), "stdout", "-l", language]
    if oem is not None:
        command.extend(["--oem", str(oem)])
    if psm is not None:
        command.extend(["--psm", str(psm)])
    for key, value in (configs or {}).items():
        command.extend(["-c", f"{key}={value}"])
    if output_format:
        command.append(output_format)
    return command


def parse_tesseract_tsv_result(
    tsv_text: str,
    *,
    min_line_confidence: float | None = DEFAULT_MIN_LINE_CONFIDENCE,
) -> ParsedOcrText:
    lines: dict[tuple[int, int, int, int], list[dict[str, object]]] = {}
    reader = csv.DictReader(tsv_text.splitlines(), delimiter="\t", quoting=csv.QUOTE_NONE)
    for row in reader:
        if row.get("level") != "5":
            continue
        text = (row.get("text") or "").strip()
        if not text:
            continue
        try:
            key = (
                int(row.get("page_num") or 0),
                int(row.get("block_num") or 0),
                int(row.get("par_num") or 0),
                int(row.get("line_num") or 0),
            )
            word = {
                "left": int(row.get("left") or 0),
                "top": int(row.get("top") or 0),
                "width": int(row.get("width") or 0),
                "height": int(row.get("height") or 0),
                "conf": _parse_confidence(row.get("conf")),
                "text": text,
            }
        except ValueError:
            continue
        lines.setdefault(key, []).append(word)

    rendered: list[str] = []
    confidence_total = 0.0
    confidence_weight = 0
    for key in sorted(lines):
        words = sorted(lines[key], key=lambda word: (int(word["left"]), int(word["top"])))
        line_confidence = _line_confidence(words)
        if not _line_confidence_passes(line_confidence, min_line_confidence):
            continue
        line = _join_ocr_words(words)
        if _is_likely_noise_line(line):
            continue
        if line:
            rendered.append(line)
            weight = max(len(line), 1)
            confidence_total += line_confidence * weight
            confidence_weight += weight
    confidence = confidence_total / confidence_weight if confidence_weight else 0.0
    return ParsedOcrText("\n".join(rendered), confidence)


def parse_tesseract_tsv(tsv_text: str, *, min_line_confidence: float | None = DEFAULT_MIN_LINE_CONFIDENCE) -> str:
    return parse_tesseract_tsv_result(tsv_text, min_line_confidence=min_line_confidence).text


def _parse_confidence(value: str | None) -> float:
    try:
        return float(value or 0)
    except ValueError:
        return 0.0


def _line_confidence(words: list[dict[str, object]]) -> float:
    total_weight = 0
    weighted_confidence = 0.0
    for word in words:
        text = str(word["text"])
        weight = max(len(text), 1)
        total_weight += weight
        weighted_confidence += max(float(word["conf"]), 0.0) * weight
    if not total_weight:
        return 0.0
    return weighted_confidence / total_weight


def _line_confidence_passes(line_confidence: float, min_line_confidence: float | None) -> bool:
    if min_line_confidence is None or min_line_confidence < 0:
        return True
    return line_confidence >= min_line_confidence


def _is_likely_noise_line(line: str) -> bool:
    text = line.strip()
    if not text:
        return False
    cjk_count = sum(1 for char in text if _is_cjk(char))
    digit_count = sum(1 for char in text if char.isdigit())
    marker_count = sum(1 for char in text if char in "|~`^\\")
    if marker_count and cjk_count < 2:
        return True
    if cjk_count == 0 and digit_count and len(text) <= 8:
        return True
    return False


def _join_ocr_words(words: list[dict[str, object]]) -> str:
    pieces: list[str] = []
    previous: dict[str, object] | None = None
    for word in words:
        text = str(word["text"])
        if previous is not None and _needs_space_between(previous, word):
            pieces.append(" ")
        pieces.append(text)
        previous = word
    return "".join(pieces).strip()


def _needs_space_between(previous: dict[str, object], current: dict[str, object]) -> bool:
    previous_text = str(previous["text"])
    current_text = str(current["text"])
    if not previous_text or not current_text:
        return False
    if _is_cjk(previous_text[-1]) and _is_cjk(current_text[0]):
        return False
    if current_text[0] in ",.;:!?%)]}，。；：！？、）】》":
        return False
    gap = int(current["left"]) - (int(previous["left"]) + int(previous["width"]))
    height = max(int(previous["height"]), int(current["height"]), 1)
    return gap > height * 0.2


def _is_cjk(char: str) -> bool:
    return (
        "\u3400" <= char <= "\u4dbf"
        or "\u4e00" <= char <= "\u9fff"
        or "\uf900" <= char <= "\ufaff"
    )


def _lanczos_resampling(image_module: object) -> object:
    resampling = getattr(image_module, "Resampling", None)
    if resampling is not None:
        return resampling.LANCZOS
    return getattr(image_module, "LANCZOS")


def preprocess_image_for_ocr(image_path: Path, output_dir: Path, *, verbose: bool = False) -> Path:
    try:
        from PIL import Image, ImageEnhance, ImageOps
    except ImportError:
        if verbose:
            print("Pillow is not available; OCR preprocessing skipped.", file=sys.stderr)
        return image_path

    try:
        with Image.open(image_path) as image:
            image = image.convert("RGB")
            width, height = image.size
            if width > 0 and height > 0:
                image = image.resize(
                    (width * PREPROCESS_SCALE, height * PREPROCESS_SCALE),
                    _lanczos_resampling(Image),
                )
            grayscale = ImageOps.grayscale(image)
            enhanced = ImageEnhance.Contrast(grayscale).enhance(PREPROCESS_CONTRAST)
            prepared = enhanced.point(
                lambda pixel: 0 if pixel < PREPROCESS_THRESHOLD else 255,
                mode="1",
            )
            output_path = output_dir / f"{image_path.stem}_ocr_preprocessed.png"
            prepared.save(output_path)
            return output_path
    except Exception as exc:
        if verbose:
            print(f"OCR preprocessing skipped for {image_path}: {exc}", file=sys.stderr)
        return image_path


def output_path_for(
    image_paths: list[Path],
    output: str | None,
    output_dir: str | None,
    *,
    output_stem: str | None = None,
    suffix: str = DEFAULT_SUFFIX,
) -> Path:
    if output:
        path = Path(output).expanduser()
        if path.suffix.lower() != ".txt":
            path = path.with_suffix(".txt")
        return path

    if not image_paths:
        raise ImageOcrError("No image files were provided.")
    parent = Path(output_dir).expanduser() if output_dir else image_paths[0].parent
    if output_stem:
        stem = Path(output_stem).stem
    elif len(image_paths) == 1:
        stem = image_paths[0].stem
    else:
        stem = "images"
    return parent / f"{stem}{suffix}.txt"


def tesseract_ocr_image(
    image_path: Path,
    *,
    tesseract_bin: str | None = None,
    language: str = DEFAULT_LANGUAGE,
    psm: int | None = DEFAULT_PSM,
    preprocess: bool = DEFAULT_PREPROCESS,
    min_line_confidence: float | None = DEFAULT_MIN_LINE_CONFIDENCE,
    verbose: bool = False,
) -> OcrResult:
    if not image_path.exists():
        raise ImageOcrError(f"Input image does not exist: {image_path}")

    executable = Path(tesseract_bin).expanduser() if tesseract_bin else Path(require_binary("tesseract"))
    if tesseract_bin and not executable.exists():
        found = shutil.which(tesseract_bin)
        if not found:
            raise ImageOcrError(f"tesseract binary was not found: {tesseract_bin}")
        executable = Path(found)

    with ExitStack() as stack:
        image_candidates = [image_path]
        if preprocess:
            temp_dir = Path(stack.enter_context(tempfile.TemporaryDirectory()))
            prepared = preprocess_image_for_ocr(image_path, temp_dir, verbose=verbose)
            if prepared != image_path:
                image_candidates.append(prepared)

        ocr_candidates = [
            _run_tesseract_tsv(
                executable,
                candidate_path,
                original_path=image_path,
                language=language,
                psm=psm,
                min_line_confidence=min_line_confidence,
                verbose=verbose,
            )
            for candidate_path in image_candidates
        ]

    best = max(ocr_candidates, key=_ocr_candidate_score)
    return OcrResult(image_path, normalize_ocr_text(best.text))


def _run_tesseract_tsv(
    executable: Path,
    image_path: Path,
    *,
    original_path: Path,
    language: str,
    psm: int | None,
    min_line_confidence: float | None,
    verbose: bool,
) -> ParsedOcrText:
    command = build_tesseract_command(
        executable,
        image_path,
        language=language,
        psm=psm,
        oem=DEFAULT_OEM,
        configs={"preserve_interword_spaces": "1"},
        output_format="tsv",
    )
    if verbose:
        print(" ".join(command), file=sys.stderr)
    completed = subprocess.run(command, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        detail = completed.stderr.strip()
        raise ImageOcrError(
            f"tesseract failed for {original_path} with exit code {completed.returncode}"
            + (f": {detail}" if detail else "")
        )
    return parse_tesseract_tsv_result(completed.stdout, min_line_confidence=min_line_confidence)


def _ocr_candidate_score(candidate: ParsedOcrText) -> float:
    text = candidate.text
    cjk_count = sum(1 for char in text if _is_cjk(char))
    latin_count = sum(1 for char in text if char.isascii() and char.isalpha())
    marker_count = sum(1 for char in text if char in "|~`^\\")
    return candidate.confidence + min(cjk_count, 200) * 0.03 - latin_count * 0.4 - marker_count * 1.5


def render_ocr_results(results: list[OcrResult]) -> str:
    if not results:
        return ""
    if len(results) == 1:
        text = results[0].text.strip()
        return f"{text}\n" if text else ""

    chunks: list[str] = []
    for result in results:
        chunks.append(f"## {result.path}")
        if result.text.strip():
            chunks.append(result.text.strip())
        chunks.append("")
    return "\n".join(chunks).rstrip() + "\n"


def render_ocr_progress_bar(
    completed: int,
    total: int,
    *,
    current: Path | None = None,
    width: int = 30,
) -> str:
    safe_total = max(1, total)
    safe_completed = max(0, min(completed, safe_total))
    percent = round(100 * safe_completed / safe_total)
    filled = round(width * percent / 100)
    line = (
        f"ocr_progress: [{'#' * filled}{'.' * (width - filled)}] "
        f"{percent:3d}% ({safe_completed}/{total})"
    )
    if current is not None:
        line += f" processing={current.name}"
    return line


def print_ocr_progress_bar(
    completed: int,
    total: int,
    *,
    current: Path | None = None,
    interactive: bool,
) -> None:
    line = render_ocr_progress_bar(completed, total, current=current)
    if interactive:
        progress_writer = getattr(sys.stderr, "write_progress", None)
        if callable(progress_writer):
            progress_writer(line)
            return
        print(f"\r{line}", end="", file=sys.stderr, flush=True)
        return
    print(line, file=sys.stderr, flush=True)


def finish_ocr_progress_bar(*, interactive: bool) -> None:
    if interactive:
        progress_finisher = getattr(sys.stderr, "finish_progress", None)
        if callable(progress_finisher):
            progress_finisher()
            return
        print(file=sys.stderr, flush=True)


def ocr_images(
    image_paths: list[Path],
    *,
    output: str | None = None,
    output_dir: str | None = None,
    output_stem: str | None = None,
    tesseract_bin: str | None = None,
    language: str = DEFAULT_LANGUAGE,
    psm: int | None = DEFAULT_PSM,
    preprocess: bool = DEFAULT_PREPROCESS,
    min_line_confidence: float | None = DEFAULT_MIN_LINE_CONFIDENCE,
    overwrite: bool = False,
    verbose: bool = False,
    print_progress: bool = False,
) -> Path:
    if not image_paths:
        raise ImageOcrError("No image files were provided.")
    output_path = output_path_for(image_paths, output, output_dir, output_stem=output_stem)
    if output_path.exists() and not overwrite:
        raise ImageOcrError(f"OCR output already exists, pass --overwrite to replace it: {output_path}")

    results: list[OcrResult] = []
    total = len(image_paths)
    interactive_progress = sys.stderr.isatty()
    if print_progress:
        print_ocr_progress_bar(
            0,
            total,
            current=image_paths[0],
            interactive=interactive_progress,
        )
    try:
        for index, path in enumerate(image_paths, start=1):
            results.append(
                tesseract_ocr_image(
                    path,
                    tesseract_bin=tesseract_bin,
                    language=language,
                    psm=psm,
                    preprocess=preprocess,
                    min_line_confidence=min_line_confidence,
                    verbose=verbose,
                )
            )
            if print_progress:
                next_path = image_paths[index] if index < total else None
                print_ocr_progress_bar(
                    index,
                    total,
                    current=next_path,
                    interactive=interactive_progress,
                )
    finally:
        if print_progress:
            finish_ocr_progress_bar(interactive=interactive_progress)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(render_ocr_results(results), encoding="utf-8")
    return output_path


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run OCR on local image files with Tesseract.")
    parser.add_argument("input", nargs="*", help="Input image files. Defaults to latest image in downloads/.")
    parser.add_argument(
        "--downloads-dir",
        default=DEFAULT_DOWNLOAD_DIR,
        help="Directory used when input is omitted. Default: downloads",
    )
    parser.add_argument("-o", "--output", help="Output TXT path. Default: image stem plus _ocr.txt")
    parser.add_argument("--output-dir", help="Directory for default OCR output. Default: input image directory")
    parser.add_argument("--tesseract-bin", help="Path or executable name for tesseract.")
    parser.add_argument("--language", default=DEFAULT_LANGUAGE, help=f"Tesseract language list. Default: {DEFAULT_LANGUAGE}")
    parser.add_argument(
        "--psm",
        type=int,
        default=DEFAULT_PSM,
        help=f"Tesseract page segmentation mode. Default: {DEFAULT_PSM}",
    )
    parser.add_argument(
        "--min-line-confidence",
        type=float,
        default=DEFAULT_MIN_LINE_CONFIDENCE,
        help=(
            "Drop OCR lines whose weighted confidence is below this value. "
            f"Default: {DEFAULT_MIN_LINE_CONFIDENCE}; set below 0 to disable."
        ),
    )
    preprocess_group = parser.add_mutually_exclusive_group()
    preprocess_group.add_argument(
        "--preprocess",
        dest="preprocess",
        action="store_true",
        default=DEFAULT_PREPROCESS,
        help="Try enhanced images as OCR candidates and keep the best confidence result. Default: enabled",
    )
    preprocess_group.add_argument(
        "--no-preprocess",
        dest="preprocess",
        action="store_false",
        help="Disable image enhancement before OCR.",
    )
    parser.add_argument("--overwrite", action="store_true", help="Overwrite OCR output if it already exists.")
    parser.add_argument("-v", "--verbose", action="store_true", help="Print the tesseract command.")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        image_paths = [Path(path).expanduser() for path in args.input]
        if not image_paths:
            image_paths = [latest_image(Path(args.downloads_dir).expanduser())]
        output_path = ocr_images(
            image_paths,
            output=args.output,
            output_dir=args.output_dir,
            tesseract_bin=args.tesseract_bin,
            language=args.language,
            psm=args.psm,
            preprocess=args.preprocess,
            min_line_confidence=args.min_line_confidence,
            overwrite=args.overwrite,
            verbose=args.verbose,
            print_progress=True,
        )
        print(f"ocr: {output_path}")
        return 0
    except (ImageOcrError, OSError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
