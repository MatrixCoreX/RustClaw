"""Bind native UI test results to the package manifest after tests complete."""
import json
import os
from pathlib import Path

root = Path(__file__).resolve().parents[1]
manifest_path = root / "releases/native/manifest.json"
manifest = json.loads(manifest_path.read_text())
if os.environ["DESKTOP_PACKAGE_PLATFORM"] == "windows-x64":
    report = json.loads((root / "test-results/native-platform/e2e/report.json").read_text())
    expected = root / "test-results/native-platform/Installed app 测试/agent-desktop.exe"
    assert report["ok"] and Path(report["binary"]).resolve() == expected.resolve()
    manifest["native_ui_checks"] = report["checks"]
    manifest["limits"].remove("full_native_ui_e2e_pending")
manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
