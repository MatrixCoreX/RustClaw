#!/usr/bin/env python3
"""Create a relocatable macOS tool bundle for repository-managed skills."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile


SYSTEM_PREFIXES = ("/System/Library/", "/usr/lib/")


def run(*command: str) -> str:
    completed = subprocess.run(command, check=True, capture_output=True, text=True)
    return completed.stdout if completed.stdout.strip() else completed.stderr


def first_output_line(*command: str) -> str:
    output = run(*command).strip()
    return output.splitlines()[0] if output else Path(command[0]).name


def is_macho(path: Path) -> bool:
    if not path.is_file() or path.is_symlink():
        return False
    with path.open("rb") as stream:
        magic = stream.read(4)
    return magic in {
        b"\xfe\xed\xfa\xce",
        b"\xce\xfa\xed\xfe",
        b"\xfe\xed\xfa\xcf",
        b"\xcf\xfa\xed\xfe",
        b"\xca\xfe\xba\xbe",
        b"\xbe\xba\xfe\xca",
        b"\xca\xfe\xba\xbf",
        b"\xbf\xba\xfe\xca",
    }


def linked_libraries(path: Path) -> list[str]:
    lines = run("otool", "-L", str(path)).splitlines()[1:]
    return [line.strip().split(" (", 1)[0] for line in lines if line.strip()]


def load_rpaths(path: Path) -> list[str]:
    lines = run("otool", "-l", str(path)).splitlines()
    values: list[str] = []
    for index, line in enumerate(lines):
        if line.strip() != "cmd LC_RPATH":
            continue
        for candidate in lines[index + 1 : index + 8]:
            match = re.match(r"\s*path\s+(.+?)\s+\(offset\s+\d+\)", candidate)
            if match:
                values.append(match.group(1))
                break
    return values


def resolve_dependency(dependency: str, origin: Path) -> Path | None:
    if dependency.startswith(SYSTEM_PREFIXES):
        return None
    if dependency.startswith("/"):
        path = Path(dependency)
        if path.is_file():
            return path.resolve()
        raise ValueError(f"macos_dependency_missing:{dependency}")
    if dependency.startswith("@loader_path/"):
        path = origin.parent / dependency.removeprefix("@loader_path/")
        return path.resolve() if path.is_file() else None
    if dependency.startswith("@executable_path/"):
        return None
    if dependency.startswith("@rpath/"):
        suffix = dependency.removeprefix("@rpath/")
        for value in load_rpaths(origin):
            value = value.replace("@loader_path", str(origin.parent))
            value = value.replace("@executable_path", str(origin.parent))
            candidate = Path(value) / suffix
            if candidate.is_file():
                return candidate.resolve()
        for root in (Path("/opt/homebrew/lib"), Path("/usr/local/lib")):
            candidate = root / suffix
            if candidate.is_file():
                return candidate.resolve()
        raise ValueError(f"macos_rpath_dependency_missing:{origin}:{dependency}")
    return None


def copy_tree_without_links(source: Path, destination: Path) -> None:
    shutil.copytree(source, destination, symlinks=False)
    links = [path for path in destination.rglob("*") if path.is_symlink()]
    if links:
        raise ValueError(f"macos_tool_bundle_symlink:{links[0]}")


def adhoc_sign_macho_files(root: Path) -> None:
    binaries = sorted(path for path in root.rglob("*") if is_macho(path))
    if not binaries:
        raise ValueError(f"macos_macho_bundle_empty:{root}")
    with tempfile.TemporaryDirectory(prefix="macos-macho-sign-") as temporary:
        signing_root = Path(temporary)
        for index, binary in enumerate(binaries):
            binary.chmod(binary.stat().st_mode | stat.S_IWUSR)
            staged = signing_root / f"{index}-{binary.name}"
            shutil.copy2(binary, staged)
            subprocess.run(
                [
                    "codesign",
                    "--force",
                    "--sign",
                    "-",
                    "--timestamp=none",
                    "--preserve-metadata=identifier,entitlements,flags,runtime",
                    str(staged),
                ],
                check=True,
            )
            subprocess.run(
                ["codesign", "--verify", "--strict", str(staged)],
                check=True,
            )
            shutil.copy2(staged, binary)


def tool_launcher_source(chromium_relative: str) -> str:
    chromium_suffix = json.dumps("/" + chromium_relative)
    return rf"""#include <errno.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int join_path(char *output, size_t output_size, const char *root, const char *suffix) {{
    int written = snprintf(output, output_size, "%s%s", root, suffix);
    return written >= 0 && (size_t)written < output_size;
}}

int main(int argc, char **argv) {{
    char executable[PATH_MAX];
    uint32_t executable_size = sizeof(executable);
    if (argc < 1 || _NSGetExecutablePath(executable, &executable_size) != 0) {{
        fprintf(stderr, "bundled_tool_launcher_path_unavailable\n");
        return 127;
    }}
    char resolved[PATH_MAX];
    if (realpath(executable, resolved) == NULL) {{
        fprintf(stderr, "bundled_tool_launcher_realpath:%s\n", strerror(errno));
        return 127;
    }}
    char tool_name[64];
    char *name_separator = strrchr(resolved, '/');
    if (name_separator == NULL || snprintf(tool_name, sizeof(tool_name), "%s", name_separator + 1) >= (int)sizeof(tool_name)) {{
        fprintf(stderr, "bundled_tool_launcher_name_invalid\n");
        return 127;
    }}
    *name_separator = '\0';
    char *bin_separator = strrchr(resolved, '/');
    if (bin_separator == NULL || strcmp(bin_separator + 1, "bin") != 0) {{
        fprintf(stderr, "bundled_tool_launcher_layout_invalid\n");
        return 127;
    }}
    *bin_separator = '\0';

    const char *suffix = NULL;
    char git_exec_path[PATH_MAX];
    char git_template_dir[PATH_MAX];
    char tessdata_prefix[PATH_MAX];
    if (strcmp(tool_name, "git") == 0) {{
        suffix = "/git/bin/git";
        if (!join_path(git_exec_path, sizeof(git_exec_path), resolved, "/git/libexec/git-core") ||
            !join_path(git_template_dir, sizeof(git_template_dir), resolved, "/git/share/git-core/templates")) {{
            fprintf(stderr, "bundled_git_path_too_long\n");
            return 127;
        }}
        setenv("GIT_EXEC_PATH", git_exec_path, 1);
        setenv("GIT_TEMPLATE_DIR", git_template_dir, 1);
    }} else if (strcmp(tool_name, "tesseract") == 0) {{
        suffix = "/bin/tesseract.real";
        if (!join_path(tessdata_prefix, sizeof(tessdata_prefix), resolved, "/share/tessdata")) {{
            fprintf(stderr, "bundled_tessdata_path_too_long\n");
            return 127;
        }}
        setenv("TESSDATA_PREFIX", tessdata_prefix, 1);
    }} else if (strcmp(tool_name, "chromium") == 0) {{
        suffix = {chromium_suffix};
    }} else {{
        fprintf(stderr, "bundled_tool_launcher_unknown:%s\n", tool_name);
        return 127;
    }}

    char target[PATH_MAX];
    if (!join_path(target, sizeof(target), resolved, suffix)) {{
        fprintf(stderr, "bundled_tool_target_path_too_long\n");
        return 127;
    }}
    char **child_argv = calloc((size_t)argc + 1, sizeof(char *));
    if (child_argv == NULL) {{
        fprintf(stderr, "bundled_tool_launcher_allocation_failed\n");
        return 127;
    }}
    child_argv[0] = target;
    for (int index = 1; index < argc; ++index) {{
        child_argv[index] = argv[index];
    }}
    execv(target, child_argv);
    fprintf(stderr, "bundled_tool_exec_failed:%s:%s\n", target, strerror(errno));
    return 127;
}}
"""


def compile_tool_launchers(root: Path, chromium_relative: str) -> None:
    source = root / ".bundled-tool-launcher.c"
    launcher = root / ".bundled-tool-launcher"
    source.write_text(tool_launcher_source(chromium_relative), encoding="utf-8")
    compiler = run("xcrun", "--find", "clang").strip()
    sdk_root = run("xcrun", "--sdk", "macosx", "--show-sdk-path").strip()
    deployment_target = os.environ.get("MACOSX_DEPLOYMENT_TARGET", "13.0")
    subprocess.run(
        [
            compiler,
            "-Os",
            "-isysroot",
            sdk_root,
            f"-mmacosx-version-min={deployment_target}",
            str(source),
            "-o",
            str(launcher),
        ],
        check=True,
    )
    launcher.chmod(launcher.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    for name in ("git", "tesseract", "chromium"):
        shutil.copy2(launcher, root / "bin" / name)
    source.unlink()
    launcher.unlink()


def copy_external_libraries(root: Path, origins: dict[Path, Path]) -> None:
    library_root = root / "lib"
    library_root.mkdir()
    chromium_root = root / "chromium"
    queue = [
        path
        for path in root.rglob("*")
        if not path.is_relative_to(chromium_root) and is_macho(path)
    ]
    for path in queue:
        origins.setdefault(path, path)
    copied: dict[Path, Path] = {}
    index = 0
    while index < len(queue):
        output = queue[index]
        index += 1
        output.chmod(output.stat().st_mode | stat.S_IWUSR)
        origin = origins[output]
        changes: list[tuple[str, Path]] = []
        for dependency in linked_libraries(output):
            resolved = resolve_dependency(dependency, origin)
            if resolved is None or resolved == origin.resolve():
                continue
            bundled = copied.get(resolved)
            if bundled is None:
                digest = hashlib.sha256(str(resolved).encode()).hexdigest()[:12]
                bundled = library_root / f"{digest}-{resolved.name}"
                shutil.copy2(resolved, bundled)
                bundled.chmod(bundled.stat().st_mode | stat.S_IWUSR)
                copied[resolved] = bundled
                origins[bundled] = resolved
                queue.append(bundled)
            changes.append((dependency, bundled))
        for old, bundled in changes:
            relative = os.path.relpath(bundled, output.parent)
            subprocess.run(
                ["install_name_tool", "-change", old, f"@loader_path/{relative}", str(output)],
                check=True,
            )
        if output.parent == library_root:
            subprocess.run(
                ["install_name_tool", "-id", f"@loader_path/{output.name}", str(output)],
                check=True,
            )
    for path in queue:
        subprocess.run(["codesign", "--force", "--sign", "-", str(path)], check=True)


def app_root(executable: Path) -> Path:
    for parent in (executable, *executable.parents):
        if parent.suffix == ".app":
            return parent
    raise ValueError("macos_chromium_app_missing")


def require_file(path: Path, label: str, executable: bool = False) -> Path:
    resolved = path.expanduser().resolve(strict=True)
    if not resolved.is_file() or resolved.is_symlink():
        raise ValueError(f"macos_skill_dependency_invalid:{label}")
    if executable and not os.access(resolved, os.X_OK):
        raise ValueError(f"macos_skill_dependency_not_executable:{label}")
    return resolved


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--git-prefix", type=Path, required=True)
    parser.add_argument("--ffmpeg", type=Path, required=True)
    parser.add_argument("--ffprobe", type=Path, required=True)
    parser.add_argument("--tesseract", type=Path, required=True)
    parser.add_argument("--tessdata", type=Path, required=True)
    parser.add_argument("--chi-sim", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, required=True)
    parser.add_argument("--whisper-cli", type=Path, required=True)
    parser.add_argument("--whisper-model", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "darwin":
        raise SystemExit("macOS native publisher required")

    git_prefix = args.git_prefix.expanduser().resolve(strict=True)
    git_binary = require_file(git_prefix / "bin/git", "git", executable=True)
    ffmpeg = require_file(args.ffmpeg, "ffmpeg", executable=True)
    ffprobe = require_file(args.ffprobe, "ffprobe", executable=True)
    tesseract = require_file(args.tesseract, "tesseract", executable=True)
    whisper_cli = require_file(args.whisper_cli, "whisper-cli", executable=True)
    chi_sim = require_file(args.chi_sim, "tesseract-chi-sim")
    whisper_model = require_file(args.whisper_model, "whisper-model")
    chromium = require_file(args.chromium, "chromium", executable=True)
    tessdata = args.tessdata.expanduser().resolve(strict=True)
    if not tessdata.is_dir() or not (tessdata / "eng.traineddata").is_file():
        raise ValueError("macos_tessdata_invalid")

    destination = args.destination.expanduser().resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise ValueError(f"macos_tool_bundle_destination_exists:{destination}")
    with tempfile.TemporaryDirectory(prefix="macos-skill-tools-", dir=destination.parent) as temporary:
        root = Path(temporary) / "host"
        (root / "bin").mkdir(parents=True)
        origins: dict[Path, Path] = {}

        copy_tree_without_links(git_prefix, root / "git")
        for output in (root / "git").rglob("*"):
            if output.is_file():
                origins[output] = git_prefix / output.relative_to(root / "git")
        for source, name in (
            (ffmpeg, "ffmpeg"),
            (ffprobe, "ffprobe"),
            (tesseract, "tesseract.real"),
            (whisper_cli, "whisper-cli"),
        ):
            output = root / "bin" / name
            shutil.copy2(source, output)
            output.chmod(output.stat().st_mode | stat.S_IWUSR | stat.S_IXUSR)
            origins[output] = source

        data_root = root / "share/tessdata"
        copy_tree_without_links(tessdata, data_root)
        shutil.copy2(chi_sim, data_root / "chi_sim.traineddata")
        model_root = root / "models/whisper"
        model_root.mkdir(parents=True)
        shutil.copy2(whisper_model, model_root / "ggml-base.bin")

        chromium_source = app_root(chromium)
        chromium_target = root / "chromium/Chromium.app"
        copy_tree_without_links(chromium_source, chromium_target)
        relative_chromium = chromium.relative_to(chromium_source)
        chromium_target_executable = chromium_target / relative_chromium
        # Release archives reject symlinks, so copytree intentionally
        # materializes Chromium's framework links. The resulting outer .app is
        # no longer a canonical framework bundle and must not be passed to
        # codesign --deep. Re-sign every executable/library from a temporary
        # path outside the ambiguous bundle while preserving its identifier,
        # entitlements, and hardened-runtime flags; the functional version
        # probe below exercises the materialized layout.
        adhoc_sign_macho_files(chromium_target)

        chromium_relative = chromium_target_executable.relative_to(root).as_posix()
        compile_tool_launchers(root, chromium_relative)
        copy_external_libraries(root, origins)

        checks = {
            "git": run(str(root / "bin/git"), "--version").strip(),
            "ffmpeg": first_output_line(str(root / "bin/ffmpeg"), "-version"),
            "ffprobe": first_output_line(str(root / "bin/ffprobe"), "-version"),
            "tesseract": first_output_line(str(root / "bin/tesseract"), "--version"),
            "chromium": run(str(root / "bin/chromium"), "--version").strip(),
            "whisper_cli": first_output_line(str(root / "bin/whisper-cli"), "--help"),
        }
        languages = run(str(root / "bin/tesseract"), "--list-langs")
        if not {"eng", "chi_sim"}.issubset(set(languages.split())):
            raise ValueError("macos_tesseract_languages_missing")
        checks["tesseract_languages"] = ["chi_sim", "eng"]
        inventory = {
            "schema_version": 1,
            "tools": checks,
            "whisper_model_sha256": hashlib.sha256(
                (model_root / "ggml-base.bin").read_bytes()
            ).hexdigest(),
        }
        (root / "skill-dependencies.json").write_text(
            json.dumps(inventory, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        os.replace(root, destination)
    print(json.dumps({"ok": True, "destination": str(destination), **inventory}, sort_keys=True))


if __name__ == "__main__":
    main()
