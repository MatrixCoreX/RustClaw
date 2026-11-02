"""Prepare a signed system browser on disposable macOS GitHub runners only.

Apple QA1940: Finder/resource-fork metadata can invalidate strict codesign checks.
Never remove quarantine, re-sign an application, or relax the runtime selector.
"""
import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
CHROME = Path("/Applications/Google Chrome.app")
DETRITUS = frozenset({"com.apple.FinderInfo", "com.apple.ResourceFork"})


def remove_detritus(app: Path) -> None:
    if app.is_symlink():
        raise RuntimeError("ci_browser_bundle_symlink_rejected")
    if not app.is_dir():
        return
    for directory, _, files in os.walk(app, followlinks=False):
        for entry in [Path(directory), *(Path(directory) / name for name in files)]:
            if entry.is_symlink():
                continue
            for attribute in os.listxattr(entry, follow_symlinks=False):
                if attribute in DETRITUS:
                    os.removexattr(entry, attribute, follow_symlinks=False)


def browser_available() -> bool:
    # This is the same strict identifier/team/Apple-anchor verifier as runtime.
    return subprocess.run(
        ["/bin/bash", str(ROOT / "scripts/macos-system-browser.sh"), "--resolve-executable"],
        capture_output=True, check=False,
    ).returncode == 0


def clean_browser_metadata() -> None:
    try:
        remove_detritus(CHROME)
    except (OSError, RuntimeError):
        # Reinstall may repair permissions. The final strict verifier, not this
        # optional cleanup, is the only success condition.
        print("ci_browser_metadata_cleanup_unavailable", file=sys.stderr)


def main() -> None:
    if sys.platform != "darwin" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("ci_browser_preparation_requires_macos_github_runner")
    if browser_available():
        return
    clean_browser_metadata()
    if browser_available():
        return
    installed = subprocess.run(
        ["brew", "list", "--cask", "google-chrome"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
    ).returncode == 0
    subprocess.run(
        ["brew", "reinstall" if installed else "install", "--cask", "google-chrome"],
        env={**os.environ, "HOMEBREW_NO_AUTO_UPDATE": "1"}, check=True,
    )
    clean_browser_metadata()
    if not browser_available():
        subprocess.run(
            ["/usr/bin/codesign", "--verify", "--strict", "--verbose=2", str(CHROME)],
            check=False,
        )
        raise RuntimeError("signed_system_browser_selection_failed")


if __name__ == "__main__":
    main()
