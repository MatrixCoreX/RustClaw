#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path

SKILL_DEVELOP_DIR = Path(__file__).resolve().parents[2] / "skill_develop"
sys.path.insert(0, str(SKILL_DEVELOP_DIR))

from create_skill import registry_entry_text  # noqa: E402


class CreateSkillTest(unittest.TestCase):
    def test_on_demand_scaffold_declares_skill_store_install_metadata(self) -> None:
        entry = registry_entry_text(
            skill_name="sample_optional",
            aliases=[],
            timeout=30,
            output_kind="text",
            enabled=True,
            capabilities=[],
            on_demand=True,
        )

        self.assertIn('install_mode = "on_demand"', entry)
        self.assertIn(
            'package_manifest = "optional_skills/sample_optional/skill.toml"', entry
        )

    def test_core_scaffold_omits_skill_store_install_metadata(self) -> None:
        entry = registry_entry_text(
            skill_name="sample_core",
            aliases=[],
            timeout=30,
            output_kind="text",
            enabled=True,
            capabilities=[],
            on_demand=False,
        )

        self.assertNotIn("install_mode", entry)
        self.assertIn(
            'package_manifest = "crates/skills/sample_core/skill.toml"', entry
        )


if __name__ == "__main__":
    unittest.main()
