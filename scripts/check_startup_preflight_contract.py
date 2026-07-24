#!/usr/bin/env python3
"""Verify that binary startup preflight cannot stop a healthy deployment."""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
START_SCRIPT = ROOT / "start-all-bin.sh"


def write(path: Path, content: str, *, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    if executable:
        path.chmod(0o755)


def build_fixture(root: Path) -> Path:
    script = root / "start-all-bin.sh"
    shutil.copy2(START_SCRIPT, script)
    script.chmod(0o755)

    write(
        root / "scripts/version_info.sh",
        "print_rustclaw_version() { :; }\n",
        executable=True,
    )
    write(
        root / "stop-rustclaw.sh",
        '#!/usr/bin/env bash\nprintf "stopped\\n" > "$PWD/stop-called"\n',
        executable=True,
    )
    write(root / "configs/config.toml", "")
    write(root / "configs/channels/webd.toml", "[webd]\nenabled = false\n")
    write(
        root / "configs/channels/telegram.toml",
        "[telegram_bot]\nenabled = true\n",
    )
    write(
        root / "configs/channels/whatsapp.toml",
        "[whatsapp]\nenabled = false\n[whatsapp_web]\nenabled = false\n",
    )
    write(root / "configs/channels/wechat.toml", "[wechat]\nenabled = false\n")
    write(root / "configs/channels/feishu.toml", "[feishu]\nenabled = false\n")
    write(root / "target/release/clawd", "#!/usr/bin/env bash\nexit 0\n", executable=True)
    write(
        root / "target/release/skill-runner",
        "#!/usr/bin/env bash\nexit 0\n",
        executable=True,
    )
    return script


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="rustclaw-startup-preflight-") as raw:
        root = Path(raw)
        script = build_fixture(root)
        env = os.environ.copy()
        env["HOME"] = str(root / "home")
        env["RUSTCLAW_RUNTIME_ENV_SCRIPT"] = str(root / "missing-runtime-env.sh")
        result = subprocess.run(
            ["bash", str(script), "release"],
            cwd=root,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=20,
            check=False,
        )
        if result.returncode == 0:
            print("STARTUP_PREFLIGHT_CONTRACT failed: missing enabled binary was accepted")
            return 1
        if (root / "stop-called").exists():
            print("STARTUP_PREFLIGHT_CONTRACT failed: stop ran before preflight completed")
            return 1
        required = (
            "telegramd",
            "Startup preflight failed",
            "existing RustClaw processes were left unchanged",
        )
        missing = [token for token in required if token not in result.stdout]
        if missing:
            print(f"STARTUP_PREFLIGHT_CONTRACT failed: missing_output={missing}")
            return 1

    print("STARTUP_PREFLIGHT_CONTRACT ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
