#!/usr/bin/env python3
"""Guard Linux/macOS runtime and development-script portability contracts."""

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_FILE_TOKENS = {
    "AGENTS.md": (
        "All shared production code must support both Linux and macOS.",
        "Never attempt Linux commands implicitly on macOS",
    ),
    "crates/claw-core/src/config.rs": (
        "pub enum ToolSandboxBackend",
        "Auto",
        "Bubblewrap",
        "MacosSeatbelt",
        "RemoteContainer",
        "pub sandbox_backend: ToolSandboxBackend",
    ),
    "crates/clawd/src/process_sandbox.rs": (
        "trait ProcessSandboxBackend",
        "sandbox_backend_diagnostics",
        'Path::new("/usr/bin/sandbox-exec")',
        "build_macos_seatbelt_profile",
        '"sandbox_remote_backend_not_configured"',
        '"sandbox_backend_unavailable"',
        'resolved_backend: "unsupported"',
        "ToolSandboxMode::DangerFull",
    ),
    "crates/clawd/src/skills/builtin_run_cmd.rs": (
        "sandbox_backend: claw_core::config::ToolSandboxBackend",
        "backend: sandbox_backend",
        "command -v gtimeout",
        "subprocess.TimeoutExpired",
        "portable_timeout_backend_unavailable",
    ),
    "crates/clawd/src/skills/runner.rs": (
        "backend: state.skill_rt.tools_policy.sandbox_backend",
        "sandbox_backend_requested",
    ),
    "crates/clawd/src/agent_hooks/command.rs": (
        "sandbox_backend: ToolSandboxBackend",
        "backend: sandbox_backend",
        "ToolSandboxMode::ReadOnly",
        "ProcessNetworkPolicy::Deny",
    ),
    "crates/skills/service_control/src/platform.rs": (
        '#[cfg(target_os = "linux")]',
        "detect_linux_manager_for_target",
        "manager_supported_on_platform",
        "systemctl_output",
        "service_output",
        "sudo_systemctl_output",
        "sudo_service_output",
        'platform == "macos"',
    ),
    "crates/skills/system_basic/src/platform_helpers.rs": (
        '#[cfg(target_os = "linux")]',
        '#[cfg(not(target_os = "linux"))]',
        'run_command_lines("sysctl"',
        "memory_rss_bytes_from_ps",
    ),
    "crates/skills/health_check/src/main.rs": (
        '"linux" => parse_linux_uptime',
        '"macos" => read_command_output("sysctl", &["-n", "kern.boottime"])',
        "current_macos_memory_bytes",
        "default_service_manager",
        '"launchd".to_string()',
    ),
    "optional_skills/photo_organize/src/main.rs": (
        '#[cfg(target_os = "macos")]',
        "discover_macos_volume_roots",
        '#[cfg(target_os = "linux")]',
        "discover_linux_mountinfo_roots",
    ),
    "crates/skills/package_manager/src/main.rs": (
        '"macos" => &[',
        '"brew", "apt-get"',
        "package_manager_candidates",
    ),
    "crates/clawd/src/system_health.rs": (
        '#[cfg(target_os = "linux")]',
        '#[cfg(not(target_os = "linux"))]',
        "process_snapshots_from_linux_proc",
        "process_snapshots_from_ps",
    ),
    "crates/telegramd/src/main.rs": (
        '#[cfg(target_os = "linux")]',
        '#[cfg(not(target_os = "linux"))]',
        'std::process::Command::new("ps")',
    ),
    "crates/clawd/src/http/ui_routes/service_control.rs": (
        '#[cfg(target_os = "linux")]',
        '#[cfg(not(target_os = "linux"))]',
        "fn raspberry_pi_model() -> Option<String>",
    ),
    "scripts/shell_compat.sh": (
        "run_with_timeout()",
        "timeout=timeout_seconds",
        "file_mtime_epoch()",
        "file_size_bytes()",
        "latest_tree_mtime_epoch()",
        "format_epoch_local()",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
    ),
    "scripts/nl_tests/run_manual_test.sh": (
        'source "${ROOT_DIR}/scripts/shell_compat.sh"',
        "file_mtime_epoch",
        "latest_tree_mtime_epoch",
    ),
    "scripts/clawcli_smoke.sh": (
        'source "${ROOT}/scripts/shell_compat.sh"',
        "run_with_timeout",
    ),
    "docs/cross_platform_contract.md": (
        "Linux",
        "macOS",
        "Bubblewrap",
        "Seatbelt",
        "fail closed",
    ),
}

BASH4_ONLY_PATTERNS = (
    (re.compile(r"\b(?:mapfile|readarray)\b"), "bash4_mapfile"),
    (re.compile(r"\bdeclare\s+-A\b"), "bash4_associative_array"),
    (re.compile(r"\$\{[^}\n]+,,[^}\n]*\}"), "bash4_lowercase_expansion"),
)

GNU_ONLY_PATTERNS = (
    (re.compile(r"\bstat\s+-c\b"), "gnu_stat"),
    (re.compile(r"\bdate\s+-d\b"), "gnu_date"),
    (re.compile(r"\bfind\b[^\n]*\s-printf\b"), "gnu_find_printf"),
)


def read(root: Path, relative: str) -> str:
    path = root / relative
    if not path.is_file():
        return ""
    return path.read_text(encoding="utf-8")


def evaluate(root: Path) -> list[str]:
    findings: list[str] = []
    for relative, tokens in REQUIRED_FILE_TOKENS.items():
        text = read(root, relative)
        if not text:
            findings.append(f"missing_file:{relative}")
            continue
        for token in tokens:
            if token not in text:
                findings.append(f"missing_token:{relative}:{token}")

    shell_root = root / "scripts"
    if shell_root.is_dir():
        for path in sorted(shell_root.rglob("*.sh")):
            relative = path.relative_to(root).as_posix()
            text = path.read_text(encoding="utf-8")
            for pattern, label in BASH4_ONLY_PATTERNS:
                if pattern.search(text):
                    findings.append(f"{label}:{relative}")
            for pattern, label in GNU_ONLY_PATTERNS:
                if pattern.search(text):
                    findings.append(f"{label}:{relative}")

    service_main = read(root, "crates/skills/service_control/src/main.rs")
    for executable in ("systemctl", "service", "journalctl"):
        token = f'Command::new("{executable}")'
        if token in service_main:
            findings.append(f"linux_service_launch_outside_adapter:{executable}")
    for token, label in (
        ('["-n", "systemctl"', "sudo_systemctl"),
        ('["-n", "service"', "sudo_service"),
    ):
        if token in service_main:
            findings.append(f"linux_service_launch_outside_adapter:{label}")
    return findings


def write_fixture(root: Path) -> None:
    for relative, tokens in REQUIRED_FILE_TOKENS.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(tokens) + "\n", encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="cross-platform-contract-") as tmp:
        fixture = Path(tmp)
        write_fixture(fixture)
        if findings := evaluate(fixture):
            print(f"SELF_TEST_FAIL positive findings={findings}")
            return 1

        bad = fixture / "scripts/bad.sh"
        bad.write_text("#!/usr/bin/env bash\nmapfile -t rows < input\nstat -c '%s' file\n")
        findings = evaluate(fixture)
        if not any(item.startswith("bash4_mapfile:") for item in findings):
            print(f"SELF_TEST_FAIL mapfile findings={findings}")
            return 1
        if not any(item.startswith("gnu_stat:") for item in findings):
            print(f"SELF_TEST_FAIL gnu_stat findings={findings}")
            return 1

        bad.unlink()
        service_main = fixture / "crates/skills/service_control/src/main.rs"
        service_main.write_text('Command::new("systemctl")\n', encoding="utf-8")
        findings = evaluate(fixture)
        if "linux_service_launch_outside_adapter:systemctl" not in findings:
            print(f"SELF_TEST_FAIL service findings={findings}")
            return 1

    print("CROSS_PLATFORM_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    findings = evaluate(ROOT)
    if findings:
        print(f"CROSS_PLATFORM_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print("CROSS_PLATFORM_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
