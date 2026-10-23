"""Select a supported, vendor-signed macOS browser without opening user profiles."""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import plistlib
import subprocess
import sys


MINIMUM_CHROMIUM_MAJOR = 120
BROWSERS = (
    ("Google Chrome.app", "Google Chrome", "com.google.Chrome", "EQHXZ8M8AV"),
    ("Microsoft Edge.app", "Microsoft Edge", "com.microsoft.edgemac", "UBF8T346G9"),
)


@dataclass(frozen=True)
class MacOSBrowser:
    executable: str
    bundle_id: str

    @property
    def profile_namespace(self) -> str:
        return "macos-" + self.bundle_id


def select_macos_browser(
    *, platform_name: str | None = None, applications: tuple[Path, ...] | None = None,
) -> MacOSBrowser | None:
    if (sys.platform if platform_name is None else platform_name) != "darwin":
        return None
    roots = applications or (Path.home() / "Applications", Path("/Applications"))
    for app_name, executable_name, bundle_id, team in BROWSERS:
        for root in roots:
            app = root / app_name
            executable = app / "Contents/MacOS" / executable_name
            try:
                if not executable.is_file() or not os.access(executable, os.X_OK):
                    continue
                with (app / "Contents/Info.plist").open("rb") as stream:
                    info = plistlib.load(stream)
                if not isinstance(info, dict):
                    continue
                if info.get("CFBundleIdentifier") != bundle_id:
                    continue
                if info.get("CFBundleExecutable") != executable_name:
                    continue
                major = str(info.get("CFBundleShortVersionString", "")).split(".")[0]
                if not major.isascii() or not major.isdecimal() or int(major) < MINIMUM_CHROMIUM_MAJOR:
                    continue
                requirement = (
                    f'anchor apple generic and identifier "{bundle_id}" '
                    f'and certificate leaf[subject.OU] = "{team}"'
                )
                subprocess.run(
                    ["/usr/bin/codesign", "--verify", "--strict", "-R", requirement, str(app)],
                    check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5,
                )
                return MacOSBrowser(str(executable), bundle_id)
            except (OSError, ValueError, plistlib.InvalidFileException, subprocess.SubprocessError):
                continue
    return None
