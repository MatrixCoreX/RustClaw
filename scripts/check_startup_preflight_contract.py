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
        root / "component_start/common.sh",
        (ROOT / "component_start/common.sh").read_text(encoding="utf-8"),
        executable=True,
    )
    write(
        root / "component_start/start-whisper-server.sh",
        "#!/usr/bin/env bash\necho 'local whisper fixture skipped'\nexit 0\n",
        executable=True,
    )

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
    write(root / "configs/channels/whatsapp-cloud.toml", "[whatsapp]\nenabled = false\n")
    write(
        root / "configs/channels/whatsapp-web.toml",
        "[whatsapp_web]\nenabled = false\n",
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

    with tempfile.TemporaryDirectory(prefix="rustclaw-whatsapp-preflight-") as raw:
        root = Path(raw)
        script = build_fixture(root)
        write(
            root / "configs/channels/telegram.toml",
            "[telegram_bot]\nenabled = false\n",
        )
        write(
            root / "configs/channels/whatsapp-web.toml",
            "[whatsapp_web]\nenabled = true\n",
        )
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
        if result.returncode == 0 or "whatsapp_webd" not in result.stdout:
            print(
                "STARTUP_PREFLIGHT_CONTRACT failed: split WhatsApp Web config "
                "did not require whatsapp_webd"
            )
            return 1
        if (root / "stop-called").exists():
            print(
                "STARTUP_PREFLIGHT_CONTRACT failed: stop ran before "
                "WhatsApp Web preflight completed"
            )
            return 1

    with tempfile.TemporaryDirectory(prefix="rustclaw-process-identity-") as raw:
        root = Path(raw)
        script = build_fixture(root)
        write(
            root / "configs/channels/telegram.toml",
            "[telegram_bot]\nenabled = false\n",
        )
        write(
            root / "target/release/clawd",
            "#!/usr/bin/env bash\nwhile true; do sleep 1; done\n",
            executable=True,
        )
        decoy = subprocess.Popen(
            [
                "bash",
                "-c",
                "sleep 30",
                f"mentions-but-is-not-{root / 'target/release/clawd'}",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        started_pid = 0
        try:
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
            pid_path = root / ".pids/clawd.pid"
            if pid_path.exists():
                started_pid = int(pid_path.read_text(encoding="utf-8").strip())
            if result.returncode != 0 or started_pid <= 0:
                print(
                    "STARTUP_PREFLIGHT_CONTRACT failed: exact process identity "
                    "case did not start clawd"
                )
                return 1
            if "clawd is already running" in result.stdout:
                print(
                    "STARTUP_PREFLIGHT_CONTRACT failed: an unrelated command "
                    "argument caused a clawd false positive"
                )
                return 1
        finally:
            if started_pid > 0:
                try:
                    os.kill(started_pid, 15)
                except ProcessLookupError:
                    pass
            decoy.terminate()
            try:
                decoy.wait(timeout=3)
            except subprocess.TimeoutExpired:
                decoy.kill()

    if Path("/proc/self/exe").is_symlink():
        with tempfile.TemporaryDirectory(prefix="rustclaw-relative-process-") as raw:
            root = Path(raw)
            script = build_fixture(root)
            write(
                root / "configs/channels/telegram.toml",
                "[telegram_bot]\nenabled = false\n",
            )
            shutil.copy2(shutil.which("sh") or "/bin/sh", root / "target/release/clawd")
            (root / "target/release/clawd").chmod(0o755)
            relative_clawd = subprocess.Popen(
                ["target/release/clawd", "-c", "sleep 30"],
                cwd=root,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            try:
                pid_path = root / ".pids/clawd.pid"
                pid_path.parent.mkdir(parents=True, exist_ok=True)
                pid_path.write_text(f"{relative_clawd.pid}\n", encoding="utf-8")
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
                if result.returncode != 0 or "clawd is already running" not in result.stdout:
                    print(
                        "STARTUP_PREFLIGHT_CONTRACT failed: workspace-relative "
                        "clawd process was not recognized"
                    )
                    return 1
            finally:
                relative_clawd.terminate()
                try:
                    relative_clawd.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    relative_clawd.kill()

    with tempfile.TemporaryDirectory(prefix="rustclaw-channel-isolation-") as raw:
        root = Path(raw)
        script = build_fixture(root)
        write(root / "configs/channels/webd.toml", "[webd]\nenabled = true\n")
        write(
            root / "target/release/clawd",
            "#!/usr/bin/env bash\nwhile true; do sleep 1; done\n",
            executable=True,
        )
        write(
            root / "target/release/webd",
            "#!/usr/bin/env bash\nexit 1\n",
            executable=True,
        )
        write(
            root / "target/release/telegramd",
            "#!/usr/bin/env bash\nwhile true; do sleep 1; done\n",
            executable=True,
        )
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
        started_pids: list[int] = []
        try:
            for component in ("clawd", "telegramd"):
                pid_path = root / f".pids/{component}.pid"
                if pid_path.exists():
                    started_pids.append(int(pid_path.read_text(encoding="utf-8").strip()))
            if result.returncode == 0:
                print("STARTUP_PREFLIGHT_CONTRACT failed: channel failure was hidden")
                return 1
            telegram_pid_path = root / ".pids/telegramd.pid"
            if not telegram_pid_path.exists():
                print(
                    "STARTUP_PREFLIGHT_CONTRACT failed: a webd failure blocked "
                    "the remaining telegramd startup"
                )
                return 1
            telegram_pid = int(telegram_pid_path.read_text(encoding="utf-8").strip())
            os.kill(telegram_pid, 0)
            required = (
                "continuing with remaining components: webd",
                "Startup completed with failed components: webd",
            )
            missing = [token for token in required if token not in result.stdout]
            if missing:
                print(
                    "STARTUP_PREFLIGHT_CONTRACT failed: channel isolation "
                    f"output missing={missing}"
                )
                return 1
        finally:
            for pid in started_pids:
                try:
                    os.kill(pid, 15)
                except ProcessLookupError:
                    pass

    print("STARTUP_PREFLIGHT_CONTRACT ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
