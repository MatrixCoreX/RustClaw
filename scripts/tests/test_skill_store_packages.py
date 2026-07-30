#!/usr/bin/env python3

from __future__ import annotations

import argparse
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS_DIR))

from skill_store_packages import (  # noqa: E402
    arch_for_target,
    on_demand_pairs,
    on_demand_specs,
    platform_for_target,
    runner_specs,
    select_specs,
)


class SkillStorePackagesTest(unittest.TestCase):
    @staticmethod
    def _manifest(
        root: Path,
        name: str,
        *,
        package: str | None = None,
        binary: str | None = None,
        supported_os: tuple[str, ...] = ("linux", "macos"),
        supported_arch: tuple[str, ...] = ("x86_64", "aarch64"),
    ) -> str:
        manifest = root / "skills" / name / "skill.toml"
        manifest.parent.mkdir(parents=True, exist_ok=True)
        cargo_package = package or f"{name.replace('_', '-')}-skill"
        cargo_binary = binary or cargo_package
        manifest.write_text(
            f'''schema_version = 1
[package]
name = "{name}"
version = "1.0.0"
description = "fixture"
protocol = "agent-jsonl-v1"
supported_os = {list(supported_os)!r}
supported_arch = {list(supported_arch)!r}
license = "MIT"
[registry]
name = "{name}"
[build]
adapter = "cargo"
package = "{cargo_package}"
binary = "{cargo_binary}"
[run]
launcher = "native"
entrypoint = "runtime/bin/{cargo_binary}"
[security]
capability_policy_source = "registry"
inherit_credentials = false
'''.replace("'", '"'),
            encoding="utf-8",
        )
        return manifest.relative_to(root).as_posix()

    @staticmethod
    def _selection(
        registry: Path, scope: str, target: str, skill: str = ""
    ) -> list[str]:
        args = argparse.Namespace(
            registry=registry,
            scope=scope,
            target=target,
            skill=skill,
            format="specs",
        )
        return [spec.skill_name for spec in select_specs(args)]

    def test_lists_only_on_demand_packages_and_conventional_runners(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = root / "registry.toml"
            sample_manifest = self._manifest(
                root,
                "sample_optional",
                package="sample-package",
                binary="sample-optional-skill",
            )
            custom_manifest = self._manifest(
                root,
                "custom_optional",
                package="custom-runner-skill",
                binary="custom-runner-skill",
            )
            registry.write_text(
                f"""

[[skills]]
name = "sample_optional"
kind = "runner"
install_mode = "on_demand"
package_manifest = "{sample_manifest}"

[[skills]]
name = "custom_optional"
kind = "runner"
install_mode = "on_demand"
package_manifest = "{custom_manifest}"
""",
                encoding="utf-8",
            )

            self.assertEqual(
                on_demand_pairs(registry),
                [
                    ("custom-runner-skill", "custom-runner-skill"),
                    ("sample-package", "sample-optional-skill"),
                ],
            )
            self.assertEqual(
                [spec.source_dir.as_posix() for spec in on_demand_specs(registry)],
                [
                    (root / "skills/custom_optional").as_posix(),
                    (root / "skills/sample_optional").as_posix(),
                ],
            )

    def test_target_triples_map_to_runtime_platform_names(self) -> None:
        self.assertEqual(platform_for_target("x86_64-unknown-linux-gnu"), "linux")
        self.assertEqual(platform_for_target("aarch64-apple-darwin"), "macos")
        self.assertEqual(platform_for_target("x86_64-pc-windows-msvc"), "windows")
        self.assertEqual(arch_for_target("x86_64-unknown-linux-gnu"), "x86_64")
        self.assertEqual(arch_for_target("aarch64-apple-darwin"), "aarch64")
        with self.assertRaisesRegex(ValueError, "cannot determine platform"):
            platform_for_target("mystery-target")

    def test_platform_filters_proactive_builds_but_always_excludes_on_demand(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = root / "registry.toml"
            linux_manifest = self._manifest(root, "linux_fixed", supported_os=("linux",))
            mac_manifest = self._manifest(root, "mac_fixed", supported_os=("macos",))
            optional_manifest = self._manifest(root, "portable_optional")
            registry.write_text(
                f"""
[[skills]]
name = "linux_fixed"
kind = "runner"
package_manifest = "{linux_manifest}"

[[skills]]
name = "mac_fixed"
kind = "runner"
package_manifest = "{mac_manifest}"

[[skills]]
name = "portable_optional"
kind = "runner"
install_mode = "on_demand"
package_manifest = "{optional_manifest}"
""",
                encoding="utf-8",
            )

            self.assertEqual(
                self._selection(registry, "proactive", "aarch64-unknown-linux-gnu"),
                ["linux_fixed"],
            )
            self.assertEqual(
                self._selection(registry, "proactive", "aarch64-apple-darwin"),
                ["mac_fixed"],
            )
            self.assertEqual(
                self._selection(registry, "build-excludes", "linux"),
                ["mac_fixed", "portable_optional"],
            )

    def test_selected_scope_resolves_alias_and_rejects_unsupported_platform(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = root / "registry.toml"
            manifest = self._manifest(
                root,
                "mac_runner",
                package="mac-package",
                binary="mac-custom-skill",
                supported_os=("macos",),
            )
            registry.write_text(
                f"""
[[skills]]
name = "mac_runner"
kind = "runner"
aliases = ["mac_alias"]
package_manifest = "{manifest}"
""",
                encoding="utf-8",
            )

            self.assertEqual(
                self._selection(
                    registry, "selected", "aarch64-apple-darwin", "mac_alias"
                ),
                ["mac_runner"],
            )
            spec = runner_specs(registry)[0]
            self.assertEqual((spec.package, spec.runner), ("mac-package", "mac-custom-skill"))
            with self.assertRaisesRegex(ValueError, "does not support platform linux/x86_64"):
                self._selection(registry, "selected", "linux", "mac_runner")


if __name__ == "__main__":
    unittest.main()
