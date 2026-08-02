#!/usr/bin/env python3
"""Functional regressions for split WhatsApp setup, export, and bridge state."""

from __future__ import annotations

import os
import shlex
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def run(*args: str, cwd: Path | None = None, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        args,
        cwd=cwd or ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=20,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"command failed: {args[0]} exit={completed.returncode}")
    return completed.stdout


def exported_values(output: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in output.splitlines():
        if not line.startswith("export "):
            continue
        parts = shlex.split(line)
        if len(parts) != 2 or "=" not in parts[1]:
            continue
        name, value = parts[1].split("=", 1)
        values[name] = value
    return values


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="whatsapp-split-flow-") as raw:
        fixture = Path(raw)
        web_config = fixture / "configs/channels/whatsapp-web.toml"
        write(web_config, "[whatsapp_web]\nenabled = true\n")
        state_script = ROOT / "scripts/whatsapp_web_config_state.py"
        if run("python3", str(state_script), str(web_config)).strip() != "1":
            print("WHATSAPP_SPLIT_CONFIG_FLOW failed: enabled Web config was not detected")
            return 1
        write(web_config, "[whatsapp_web]\nenabled = false\n")
        if run("python3", str(state_script), str(web_config)).strip() != "0":
            print("WHATSAPP_SPLIT_CONFIG_FLOW failed: disabled Web config was not detected")
            return 1
        write(web_config, '[whatsapp_web]\nenabled = "false"\n')
        invalid = subprocess.run(
            ["python3", str(state_script), str(web_config)],
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=20,
            check=False,
        )
        if invalid.returncode == 0:
            print("WHATSAPP_SPLIT_CONFIG_FLOW failed: non-boolean Web state was accepted")
            return 1
        write(web_config, "[whatsapp_web]\nenabled = false\n")

        write(fixture / "configs/config.toml", "")
        write(
            fixture / "configs/channels/whatsapp-cloud.toml",
            """
[whatsapp]
access_token = "split-access-placeholder"
app_secret = "split-secret-placeholder"
verify_token = "split-verify-placeholder"
phone_number_id = "split-phone-placeholder"

[whatsapp_cloud]
enabled = true
""",
        )
        write(
            fixture / "configs/channels/" / ("whatsapp" + ".toml"),
            """
[whatsapp]
access_token = "legacy-access-must-not-export"
app_secret = "legacy-secret-must-not-export"
verify_token = "legacy-verify-must-not-export"
phone_number_id = "legacy-phone-must-not-export"
""",
        )
        output = run(
            "bash",
            str(ROOT / "scripts/export_runtime_env_from_configs.sh"),
            str(fixture),
        )
        values = exported_values(output)
        expected = {
            "WHATSAPP_ACCESS_TOKEN": "split-access-placeholder",
            "WHATSAPP_APP_SECRET": "split-secret-placeholder",
            "WHATSAPP_VERIFY_TOKEN": "split-verify-placeholder",
            "WHATSAPP_PHONE_NUMBER_ID": "split-phone-placeholder",
            "WHATSAPP_CLOUD_ACCESS_TOKEN": "split-access-placeholder",
            "WHATSAPP_CLOUD_APP_SECRET": "split-secret-placeholder",
            "WHATSAPP_CLOUD_VERIFY_TOKEN": "split-verify-placeholder",
            "WHATSAPP_CLOUD_PHONE_NUMBER_ID": "split-phone-placeholder",
        }
        if any(values.get(name) != value for name, value in expected.items()):
            print("WHATSAPP_SPLIT_CONFIG_FLOW failed: Cloud export source mismatch")
            return 1

        base_config = fixture / "base.toml"
        write(base_config, "[whatsapp_web]\nenabled = true\n")
        environment = os.environ.copy()
        environment["APP_CONFIG_PATH"] = str(base_config)
        environment["APP_CHANNEL_CONFIG_DIR"] = str(fixture / "configs/channels")
        environment["APP_RUNTIME_ENV_SCRIPT"] = str(fixture / "missing-runtime-env.sh")
        bridge_output = run(
            "bash",
            str(ROOT / "component_start/start-wa-web-bridge.sh"),
            "release",
            env=environment,
        )
        if "whatsapp_web.enabled=false" not in bridge_output:
            print("WHATSAPP_SPLIT_CONFIG_FLOW failed: bridge used a non-canonical enable source")
            return 1

    print("WHATSAPP_SPLIT_CONFIG_FLOW ok cases=5")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
