"""Planner package lifecycle controls must survive registry projection."""

import tomllib
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[3]


class PackageSchemaTests(unittest.TestCase):
    def registries(self):
        for path in ("configs/skills_registry.toml", "docker/config/skills_registry.toml"):
            registry = tomllib.loads((ROOT / path).read_text())
            yield path, next(s for s in registry["skills"] if s["name"] == "package_manager")

    def test_mutations_expose_boolean_preview_control_without_granting_permissions(self):
        for path, skill in self.registries():
            for action in ("install", "smart_install", "uninstall"):
                with self.subTest(path=path, action=action):
                    cap = next(c for c in skill["planner_capabilities"] if c["action"] == action)
                    self.assertIn("dry_run", cap["optional"])
                    self.assertEqual(cap["effect"], "mutate")
                    self.assertEqual(cap["risk_level"], "high")
                    self.assertTrue(cap["package_install"])
                    self.assertTrue(cap["privilege_escalation"])
                    schema = skill["input_schema"]["properties"]["dry_run"]
                    Draft202012Validator.check_schema(schema)
                    validator = Draft202012Validator(schema)
                    for value in (True, False):
                        validator.validate(value)
                    for value in ("false", 0, None, {}):
                        self.assertTrue(list(validator.iter_errors(value)))

    def test_read_only_preview_remains_a_separate_non_mutating_capability(self):
        for path, skill in self.registries():
            with self.subTest(path=path):
                cap = next(c for c in skill["planner_capabilities"]
                           if c["action"] == "smart_install_preview")
                self.assertEqual(cap["effect"], "observe")
                self.assertEqual(cap["isolation_profile"], "read_only")
                self.assertNotIn("dry_run", cap.get("optional", []))
                self.assertFalse(cap["package_install"])
                self.assertFalse(cap["privilege_escalation"])


if __name__ == "__main__":
    unittest.main()
