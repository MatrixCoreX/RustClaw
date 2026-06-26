#!/usr/bin/env python3
"""Check timeout and resumable long-tail skill metadata contracts."""

from __future__ import annotations

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
    )
    return len(skills), [f"{path.relative_to(ROOT)}: {finding}" for finding in findings]


def main() -> int:
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
    sys.exit(main())
