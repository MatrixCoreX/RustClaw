#!/usr/bin/env python3

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS_DIR))

from skill_store_packages import on_demand_pairs, on_demand_specs  # noqa: E402


class SkillStorePackagesTest(unittest.TestCase):
    def test_lists_only_on_demand_packages_and_conventional_runners(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            registry = Path(directory) / "registry.toml"
            registry.write_text(
                """
[[skills]]
name = "always_on"

[[skills]]
name = "sample_optional"
install_mode = "on_demand"
install_package = "sample-package"

[[skills]]
name = "custom_optional"
runner_name = "custom-runner"
install_mode = "on_demand"
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
                    "optional_skills/custom_optional",
                    "optional_skills/sample_optional",
                ],
            )


if __name__ == "__main__":
    unittest.main()
