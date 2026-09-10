"""Inspect, install and launch packages on disposable native CI runners only."""
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "releases/native"
EVIDENCE = ROOT / "test-results/native-platform"


def run(args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("native_package_smoke_requires_disposable_ci_runner")
    target = os.environ["DESKTOP_NATIVE_TARGET"]
    package_platform = os.environ["DESKTOP_PACKAGE_PLATFORM"]
    commit = run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    assert commit == os.environ["SOURCE_COMMIT"]
    version = json.loads((ROOT / "package.json").read_text())["version"]
    bundle = ROOT / "target" / target / "release/bundle"
    OUT.mkdir(parents=True, exist_ok=True)
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    checks = []
    if platform.system() == "Windows":
        installers = list(bundle.glob("nsis/*-setup.exe"))
        msi = list(bundle.glob("msi/*.msi"))
        assert len(installers) == 1 and msi, "native_installers_missing"
        run(["pwsh", "-NoProfile", "-File", ROOT / "scripts/windows-package-smoke.ps1",
             "-Installer", installers[0], "-Evidence", EVIDENCE,
             "-ExpectedBinary", ROOT / "target" / target / "release/agent-desktop.exe"])
        for package in installers + msi:
            shutil.copy2(package, OUT / package.name)
        signing = "unsigned-internal-test"
        checks += ["nsis_per_user_install", "installed_binary_sha256", "native_window_open"]
    elif platform.system() == "Darwin":
        app = bundle / "macos/agent-desktop.app"
        dmg = list(bundle.glob("dmg/*.dmg"))
        assert app.is_dir() and len(dmg) == 1, "native_bundle_missing"
        info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
        assert info["CFBundleIdentifier"] == "org.agent-runtime.desktop"
        assert info["CFBundleShortVersionString"] == version
        assert info["LSMinimumSystemVersion"] == "13.0"
        assert "_agent-runtime._tcp" in info["NSBonjourServices"]
        assert info["NSLocalNetworkUsageDescription"] and info["NSMicrophoneUsageDescription"]
        ats = info["NSAppTransportSecurity"]
        assert not ats.get("NSAllowsArbitraryLoads") and not ats.get("NSAllowsArbitraryLoadsInWebContent")
        run(["codesign", "--verify", "--deep", "--strict", app])
        binary = app / "Contents/MacOS/agent-desktop"
        arch = run(["lipo", "-archs", binary], capture_output=True, text=True).stdout.strip()
        assert arch == ("arm64" if target.startswith("aarch64") else "x86_64")
        run(["hdiutil", "verify", dmg[0]])
        mount = EVIDENCE / "mounted-dmg"
        mount.mkdir(exist_ok=True)
        run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", mount, dmg[0]])
        installed = EVIDENCE / "Installed app 测试/agent-desktop.app"
        try:
            mounted = mount / "agent-desktop.app"
            assert digest(mounted / "Contents/MacOS/agent-desktop") == digest(binary)
            run(["ditto", mounted, installed])
        finally:
            run(["hdiutil", "detach", mount])
        with (EVIDENCE / "application.log").open("w") as log:
            process = subprocess.Popen([str(installed / "Contents/MacOS/agent-desktop")], stdout=log, stderr=log)
            try:
                run(["swift", ROOT / "scripts/macos-window-smoke.swift", str(process.pid)],
                    stdout=(EVIDENCE / "window.json").open("w"))
                assert process.poll() is None, "native_app_exited"
                run(["/usr/sbin/screencapture", "-x", EVIDENCE / "desktop.png"])
            finally:
                process.terminate()
                process.wait(timeout=15)
        zip_path = OUT / f"agent-desktop_{version}_{package_platform}.app.zip"
        run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, zip_path])
        shutil.copy2(dmg[0], OUT / dmg[0].name)
        signing = "ad-hoc-not-notarized-internal-test"
        checks += ["bundle_identity_permissions_architecture", "ad_hoc_signature_integrity",
                   "dmg_integrity", "installed_binary_sha256", "native_window_open"]
    else:
        raise SystemExit("native_package_platform_unsupported")
    packages = [{"file": p.name, "sha256": digest(p), "bytes": p.stat().st_size}
                for p in sorted(OUT.iterdir()) if p.suffix in {".exe", ".msi", ".dmg", ".zip"}]
    manifest = {"schema_version": 1, "version": version, "source_commit": commit,
                "target": target, "host": platform.platform(), "signing": signing,
                "checks": checks, "packages": packages,
                "limits": ["physical_lan_pairing_requires_user_machine", "full_native_ui_e2e_pending"]}
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (OUT / "SHA256SUMS").write_text("".join(f"{p['sha256']}  {p['file']}\n" for p in packages))
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
