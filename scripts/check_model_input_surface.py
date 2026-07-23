#!/usr/bin/env python3
"""Keep eager planner prompt/tool disclosure at or below the measured baseline."""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
import tempfile
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TOOL_OVERLAY = ROOT / "prompts/layers/overlays/agent_tool_spec.md"
REGISTRY = ROOT / "configs/skills_registry.toml"

# The global overlay now contains protocol only. Domain contracts belong to the
# registry and generated skill playbooks, so keep enough room for bounded
# protocol maintenance without allowing the old catalog to return.
MAX_GLOBAL_TOOL_OVERLAY_BYTES = 10_000
MAX_EAGER_NATIVE_GROUPS = 7
MAX_EAGER_PLANNER_CAPABILITIES = 71
MARKDOWN_HEADING = re.compile(r"^#{1,6}\s+(.+?)\s*#*\s*$")


@dataclasses.dataclass(frozen=True)
class SurfaceMetrics:
    global_tool_overlay_bytes: int
    eager_native_groups: int
    eager_planner_capabilities: int
    registry_skill_headings: tuple[str, ...]


def registry_surface(path: Path) -> tuple[int, int, set[str]]:
    parsed = tomllib.loads(path.read_text(encoding="utf-8"))
    groups = 0
    capabilities = 0
    skill_names: set[str] = set()
    for skill in parsed.get("skills", []):
        name = str(skill.get("name", "")).strip().lower()
        if name:
            skill_names.add(name)
        planner_capabilities = skill.get("planner_capabilities", [])
        planner_capability_aliases = skill.get("planner_capability_aliases", {})
        if (
            skill.get("enabled", True)
            and skill.get("planner_visible", True)
            and skill.get("planner_eager_load", False)
            and planner_capabilities
        ):
            groups += 1
            capabilities += sum(
                capability.get("name") not in planner_capability_aliases
                for capability in planner_capabilities
            )
    return groups, capabilities, skill_names


def registry_skill_headings(overlay_text: str, skill_names: set[str]) -> tuple[str, ...]:
    matches: set[str] = set()
    for line in overlay_text.splitlines():
        heading = MARKDOWN_HEADING.match(line.strip())
        if not heading:
            continue
        token = heading.group(1).strip().strip("`").lower()
        if token in skill_names:
            matches.add(token)
    return tuple(sorted(matches))


def inventory(tool_overlay: Path = TOOL_OVERLAY, registry: Path = REGISTRY) -> SurfaceMetrics:
    groups, capabilities, skill_names = registry_surface(registry)
    overlay_text = tool_overlay.read_text(encoding="utf-8")
    return SurfaceMetrics(
        global_tool_overlay_bytes=len(overlay_text.encode("utf-8")),
        eager_native_groups=groups,
        eager_planner_capabilities=capabilities,
        registry_skill_headings=registry_skill_headings(overlay_text, skill_names),
    )


def findings_for(metrics: SurfaceMetrics) -> list[str]:
    findings: list[str] = []
    checks = (
        (
            "global_tool_overlay_bytes_grew",
            metrics.global_tool_overlay_bytes,
            MAX_GLOBAL_TOOL_OVERLAY_BYTES,
        ),
        (
            "eager_native_groups_grew",
            metrics.eager_native_groups,
            MAX_EAGER_NATIVE_GROUPS,
        ),
        (
            "eager_planner_capabilities_grew",
            metrics.eager_planner_capabilities,
            MAX_EAGER_PLANNER_CAPABILITIES,
        ),
    )
    for token, value, ceiling in checks:
        if value > ceiling:
            findings.append(f"{token}:{value}>{ceiling}")
    findings.extend(
        f"global_tool_overlay_registry_skill_heading:{heading}"
        for heading in metrics.registry_skill_headings
    )
    return findings


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="model-input-surface-") as tmp:
        root = Path(tmp)
        overlay = root / "agent_tool_spec.md"
        registry = root / "skills_registry.toml"
        overlay.write_text("generic protocol\n", encoding="utf-8")
        registry.write_text(
            """
[[skills]]
name = "visible"
enabled = true
planner_eager_load = true
planner_capabilities = [
  { name = "demo.inspect" },
  { name = "demo.inspect_legacy" },
]
planner_capability_aliases = { "demo.inspect_legacy" = "demo.inspect" }

[[skills]]
name = "hidden"
enabled = true
planner_visible = false
planner_capabilities = [{ name = "demo.hidden" }]
""".strip()
            + "\n",
            encoding="utf-8",
        )
        measured = inventory(overlay, registry)
        assert measured == SurfaceMetrics(len(b"generic protocol\n"), 1, 1, ())
        assert not findings_for(measured)
        oversized = SurfaceMetrics(MAX_GLOBAL_TOOL_OVERLAY_BYTES + 1, 8, 72, ())
        assert findings_for(oversized) == [
            f"global_tool_overlay_bytes_grew:{MAX_GLOBAL_TOOL_OVERLAY_BYTES + 1}>{MAX_GLOBAL_TOOL_OVERLAY_BYTES}",
            "eager_native_groups_grew:8>7",
            "eager_planner_capabilities_grew:72>71",
        ]
        overlay.write_text("generic protocol\n### visible\n", encoding="utf-8")
        named_heading = inventory(overlay, registry)
        assert named_heading.registry_skill_headings == ("visible",)
        assert findings_for(named_heading) == [
            "global_tool_overlay_registry_skill_heading:visible"
        ]
    print("MODEL_INPUT_SURFACE_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    metrics = inventory()
    findings = findings_for(metrics)
    print(
        "MODEL_INPUT_SURFACE_CHECK "
        f"findings={len(findings)} "
        f"global_tool_overlay_bytes={metrics.global_tool_overlay_bytes} "
        f"eager_native_groups={metrics.eager_native_groups} "
        f"eager_planner_capabilities={metrics.eager_planner_capabilities} "
        f"registry_skill_headings={len(metrics.registry_skill_headings)}"
    )
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
