#!/usr/bin/env python3
"""Check timeout and resumable long-tail skill metadata contracts."""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = ROOT / "configs" / "skills_registry.toml"
DOCKER_REGISTRY_PATH = ROOT / "docker" / "config" / "skills_registry.toml"

MEDIA_SKILLS = {
    "image_generate": "generate",
    "audio_synthesize": "synthesize",
    "video_generate": "generate",
    "music_generate": "generate",
}
POLLABLE_MEDIA_SKILLS = {
    "image_generate": ("image", "generate"),
    "audio_synthesize": ("audio", "synthesize"),
    "video_generate": ("video", "generate"),
    "music_generate": ("music", "generate"),
}

EXECUTION_MODES = {"sync_short", "async_preferred", "async_required"}
ASYNC_EXECUTION_MODES = {"async_preferred", "async_required"}
ASYNC_ADAPTER_KINDS = {
    "local_process_poll",
    "http_job_poll",
    "mcp_job_poll",
    "media_job_poll",
    "browser_job_poll",
    "remote_job_poll",
}


def load_registry(path: Path) -> list[dict[str, Any]]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    return data.get("skills", [])


def capability_by_action(skill: dict[str, Any], action: str) -> dict[str, Any] | None:
    for cap in skill.get("planner_capabilities", []):
        if cap.get("action") == action:
            return cap
    return None


def capability_by_name(skill: dict[str, Any], name: str) -> dict[str, Any] | None:
    for cap in skill.get("planner_capabilities", []):
        if cap.get("name") == name:
            return cap
    return None


def input_properties(skill: dict[str, Any]) -> dict[str, Any]:
    schema = skill.get("input_schema")
    if not isinstance(schema, dict):
        return {}
    properties = schema.get("properties")
    return properties if isinstance(properties, dict) else {}


def required_tokens(capability: dict[str, Any]) -> set[str]:
    values = capability.get("required", [])
    if not isinstance(values, list):
        return set()
    out: set[str] = set()
    for value in values:
        if not isinstance(value, str):
            continue
        out.update(part.strip() for part in value.split("|") if part.strip())
    return out


def optional_tokens(capability: dict[str, Any]) -> set[str]:
    values = capability.get("optional", [])
    if not isinstance(values, list):
        return set()
    return {value.strip() for value in values if isinstance(value, str) and value.strip()}


def check_timeouts(skills: list[dict[str, Any]]) -> list[str]:
    findings: list[str] = []
    for skill in skills:
        name = skill.get("name", "<unknown>")
        timeout = skill.get("timeout_seconds")
        if not isinstance(timeout, int) or timeout <= 0:
            findings.append(f"{name}: timeout_seconds must be a positive integer")
    return findings


def check_capability_execution_modes(skills: list[dict[str, Any]]) -> list[str]:
    findings: list[str] = []
    for skill in skills:
        skill_name = skill.get("name", "<unknown>")
        for index, capability in enumerate(skill.get("planner_capabilities") or []):
            cap_name = capability.get("name") or f"planner_capabilities[{index}]"
            execution_mode = capability.get("execution_mode")
            if execution_mode not in EXECUTION_MODES:
                findings.append(
                    f"{skill_name}.{cap_name}: execution_mode must be one of {sorted(EXECUTION_MODES)}"
                )
                continue
            adapter_kind = capability.get("async_adapter_kind")
            if execution_mode in ASYNC_EXECUTION_MODES:
                if adapter_kind not in ASYNC_ADAPTER_KINDS:
                    findings.append(
                        f"{skill_name}.{cap_name}: async_adapter_kind must be one of {sorted(ASYNC_ADAPTER_KINDS)}"
                    )
            elif adapter_kind:
                findings.append(
                    f"{skill_name}.{cap_name}: sync_short must not declare async_adapter_kind"
                )
    return findings


def check_run_cmd_async_contract(skills_by_name: dict[str, dict[str, Any]]) -> list[str]:
    findings: list[str] = []
    skill = skills_by_name.get("run_cmd")
    if not skill:
        return ["run_cmd: missing registry skill entry"]
    props = input_properties(skill)
    for field in ["async_start", "poll_after_seconds", "expires_in_seconds"]:
        if field not in props:
            findings.append(f"run_cmd: input_schema missing {field}")
    for cap_name in ["system.run_command", "system.run_cmd"]:
        cap = capability_by_name(skill, cap_name)
        if not cap:
            findings.append(f"run_cmd: missing planner capability name={cap_name}")
            continue
        optional = optional_tokens(cap)
        for field in ["async_start", "poll_after_seconds", "expires_in_seconds"]:
            if field not in optional:
                findings.append(f"run_cmd.{cap_name}: optional missing {field}")
    return findings


def check_media_dry_run_contract(skills_by_name: dict[str, dict[str, Any]]) -> list[str]:
    findings: list[str] = []
    for skill_name, action in MEDIA_SKILLS.items():
        skill = skills_by_name.get(skill_name)
        if not skill:
            findings.append(f"{skill_name}: missing registry skill entry")
            continue
        props = input_properties(skill)
        if "dry_run" not in props:
            findings.append(f"{skill_name}: input_schema missing dry_run")
        cap = capability_by_action(skill, action)
        if not cap:
            findings.append(f"{skill_name}: missing planner capability action={action}")
            continue
        if "dry_run" not in optional_tokens(cap):
            findings.append(f"{skill_name}.{action}: optional missing dry_run")
    return findings


def check_video_poll_contract(skills_by_name: dict[str, dict[str, Any]]) -> list[str]:
    findings: list[str] = []
    skill = skills_by_name.get("video_generate")
    if not skill:
        return ["video_generate: missing registry skill entry"]
    props = input_properties(skill)
    for field in [
        "task_id",
        "job_id",
        "wait_for_completion",
        "max_poll_seconds",
        "poll_after_seconds",
        "expires_at",
        "mock_status",
        "mock_file_id",
    ]:
        if field not in props:
            findings.append(f"video_generate: input_schema missing {field}")
    generate = capability_by_action(skill, "generate")
    poll = capability_by_action(skill, "poll")
    if not generate:
        findings.append("video_generate: missing generate capability")
    else:
        optional = optional_tokens(generate)
        for field in ["wait_for_completion", "max_poll_seconds", "dry_run"]:
            if field not in optional:
                findings.append(f"video_generate.generate: optional missing {field}")
    if not poll:
        findings.append("video_generate: missing poll capability")
    else:
        if "task_id" not in required_tokens(poll):
            findings.append("video_generate.poll: required missing task_id")
        optional = optional_tokens(poll)
        for field in ["job_id", "poll_after_seconds", "expires_at", "dry_run"]:
            if field not in optional:
                findings.append(f"video_generate.poll: optional missing {field}")
        if poll.get("idempotent") is not True:
            findings.append("video_generate.poll: idempotent must be true")
    return findings


def check_pollable_media_contracts(skills_by_name: dict[str, dict[str, Any]]) -> list[str]:
    findings: list[str] = []
    for skill_name, (prefix, start_action) in POLLABLE_MEDIA_SKILLS.items():
        skill = skills_by_name.get(skill_name)
        if not skill:
            findings.append(f"{skill_name}: missing registry skill entry")
            continue
        props = input_properties(skill)
        for field in [
            "task_id",
            "job_id",
            "cancel_token",
            "cancel_ref",
            "poll_after_seconds",
            "poll_after_ms",
            "expires_at",
            "mock_status",
            "mock_file_id",
            "dry_run",
        ]:
            if field not in props:
                findings.append(f"{skill_name}: input_schema missing {field}")

        start = capability_by_action(skill, start_action)
        if not start:
            findings.append(f"{skill_name}: missing planner capability action={start_action}")
        else:
            if start.get("execution_mode") not in ASYNC_EXECUTION_MODES:
                findings.append(f"{skill_name}.{start_action}: execution_mode must be async")
            if start.get("async_adapter_kind") != "media_job_poll":
                findings.append(f"{skill_name}.{start_action}: async_adapter_kind must be media_job_poll")
            if "dry_run" not in optional_tokens(start):
                findings.append(f"{skill_name}.{start_action}: optional missing dry_run")

        poll = capability_by_name(skill, f"{prefix}.poll")
        if not poll:
            findings.append(f"{skill_name}: missing {prefix}.poll capability")
        else:
            if "task_id" not in required_tokens(poll):
                findings.append(f"{skill_name}.{prefix}.poll: required missing task_id")
            optional = optional_tokens(poll)
            for field in ["job_id", "poll_after_seconds", "poll_after_ms", "expires_at", "dry_run"]:
                if field not in optional:
                    findings.append(f"{skill_name}.{prefix}.poll: optional missing {field}")
            if poll.get("execution_mode") != "async_required":
                findings.append(f"{skill_name}.{prefix}.poll: execution_mode must be async_required")
            if poll.get("async_adapter_kind") != "media_job_poll":
                findings.append(f"{skill_name}.{prefix}.poll: async_adapter_kind must be media_job_poll")
            if poll.get("idempotent") is not True:
                findings.append(f"{skill_name}.{prefix}.poll: idempotent must be true")

        cancel = capability_by_name(skill, f"{prefix}.cancel")
        if not cancel:
            findings.append(f"{skill_name}: missing {prefix}.cancel capability")
        else:
            if "task_id" not in required_tokens(cancel):
                findings.append(f"{skill_name}.{prefix}.cancel: required missing task_id")
            optional = optional_tokens(cancel)
            for field in ["job_id", "cancel_token", "cancel_ref", "dry_run"]:
                if field not in optional:
                    findings.append(f"{skill_name}.{prefix}.cancel: optional missing {field}")
            if cancel.get("execution_mode") != "async_required":
                findings.append(f"{skill_name}.{prefix}.cancel: execution_mode must be async_required")
            if cancel.get("async_adapter_kind") != "media_job_poll":
                findings.append(f"{skill_name}.{prefix}.cancel: async_adapter_kind must be media_job_poll")
            if cancel.get("once_per_task") is not True:
                findings.append(f"{skill_name}.{prefix}.cancel: once_per_task must be true")
            if cancel.get("idempotent") is not False:
                findings.append(f"{skill_name}.{prefix}.cancel: idempotent must be false")
    return findings


def check_registry(path: Path) -> tuple[int, list[str]]:
    skills = load_registry(path)
    skills_by_name = {
        skill.get("name"): skill
        for skill in skills
        if isinstance(skill.get("name"), str)
    }
    findings = (
        check_timeouts(skills)
        + check_capability_execution_modes(skills)
        + check_run_cmd_async_contract(skills_by_name)
        + check_media_dry_run_contract(skills_by_name)
        + check_video_poll_contract(skills_by_name)
        + check_pollable_media_contracts(skills_by_name)
    )
    return len(skills), [f"{path.relative_to(ROOT)}: {finding}" for finding in findings]


def run_self_test() -> int:
    bad_timeout = [{"name": "bad_timeout", "timeout_seconds": 0}]
    if not check_timeouts(bad_timeout):
        print("SELF_TEST_FAIL missing_timeout_finding", file=sys.stderr)
        return 1

    good_sync = [
        {
            "name": "good_sync",
            "planner_capabilities": [
                {"name": "good.observe", "execution_mode": "sync_short"}
            ],
        }
    ]
    if check_capability_execution_modes(good_sync):
        print("SELF_TEST_FAIL good_sync_execution_mode_false_positive", file=sys.stderr)
        return 1

    bad_async = [
        {
            "name": "bad_async",
            "planner_capabilities": [
                {"name": "bad.start", "execution_mode": "async_required"}
            ],
        }
    ]
    if not any("async_adapter_kind" in finding for finding in check_capability_execution_modes(bad_async)):
        print("SELF_TEST_FAIL missing_async_adapter_finding", file=sys.stderr)
        return 1

    bad_media_skill = {
        "name": "image_generate",
        "input_schema": {"properties": {}},
        "planner_capabilities": [
            {
                "name": "image.generate",
                "action": "generate",
                "execution_mode": "sync_short",
                "optional": [],
            }
        ],
    }
    bad_media = {"image_generate": bad_media_skill}
    dry_run_findings = check_media_dry_run_contract(bad_media)
    pollable_findings = check_pollable_media_contracts(bad_media)
    expected_tokens = {
        "input_schema missing dry_run",
        "optional missing dry_run",
        "execution_mode must be async",
        "missing image.poll capability",
        "missing image.cancel capability",
    }
    observed_tokens = {
        token
        for finding in dry_run_findings + pollable_findings
        for token in expected_tokens
        if token in finding
    }
    if not expected_tokens.issubset(observed_tokens):
        print(
            "SELF_TEST_FAIL missing_long_tail_findings:"
            f"{dry_run_findings + pollable_findings}",
            file=sys.stderr,
        )
        return 1

    print("LONG_TAIL_SKILL_CONTRACT_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    registry_paths = [REGISTRY_PATH]
    if DOCKER_REGISTRY_PATH.exists():
        registry_paths.append(DOCKER_REGISTRY_PATH)
    findings: list[str] = []
    skill_counts: list[str] = []
    for path in registry_paths:
        skill_count, path_findings = check_registry(path)
        skill_counts.append(f"{path.relative_to(ROOT)}={skill_count}")
        findings.extend(path_findings)
    if findings:
        print(f"LONG_TAIL_SKILL_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print(
        "LONG_TAIL_SKILL_CONTRACT_CHECK ok "
        f"registries={len(registry_paths)} {' '.join(skill_counts)} media={len(MEDIA_SKILLS)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
