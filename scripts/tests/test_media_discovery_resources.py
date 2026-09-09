"""Keep collection control usable on supported low-memory devices."""

from pathlib import Path
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]


def discovery_entry(relative):
    with (ROOT / relative).open("rb") as stream:
        registry = tomllib.load(stream)
    return next(skill for skill in registry["skills"] if skill["name"] == "media_discovery")


class MediaDiscoveryResourcesTest(unittest.TestCase):
    def setUp(self):
        self.entries = [discovery_entry(path) for path in (
            "configs/skills_registry.toml", "docker/config/skills_registry.toml"
        )]

    def test_control_actions_do_not_reserve_a_browser(self):
        for entry in self.entries:
            for capability in entry["planner_capabilities"]:
                if capability["action"] in {"run_once", "run_enabled_once"}:
                    continue
                with self.subTest(action=capability["action"]):
                    request = capability.get("resource_request", entry["resource_request"])
                    self.assertEqual(request["class"], "general")
                    self.assertEqual(request["cpu_cores"], 1)
                    self.assertGreater(request["memory_mb"], 0)
                    self.assertLessEqual(request["memory_mb"], 128)
                    self.assertEqual(request.get("network_slots", 0), 0)

    def test_browser_actions_have_a_separate_positive_budget(self):
        for entry in self.entries:
            browser_actions = [cap for cap in entry["planner_capabilities"]
                               if cap["action"] in {"run_once", "run_enabled_once"}]
            self.assertEqual(len(browser_actions), 2)
            for capability in browser_actions:
                request = capability["resource_request"]
                self.assertEqual(request["class"], "network")
                self.assertEqual(request["cpu_cores"], 1)
                self.assertGreater(request["memory_mb"], entry["resource_request"]["memory_mb"])
                self.assertLessEqual(request["memory_mb"], 384)
                self.assertEqual(request["network_slots"], 1)
                self.assertEqual(capability["async_adapter_kind"], "local_process_poll")
                self.assertTrue(capability["network_access"])
                self.assertTrue(capability["subprocess"])

    def test_resource_profiles_match_container_registry(self):
        def projection(entry):
            return {cap["action"]: cap.get("resource_request", entry["resource_request"])
                    for cap in entry["planner_capabilities"]}
        self.assertEqual(projection(self.entries[0]), projection(self.entries[1]))

    def test_installable_small_devices_are_not_excluded_by_a_two_gib_floor(self):
        with (ROOT / "optional_skills/media_discovery/skill.toml").open("rb") as stream:
            manifest = tomllib.load(stream)
        minimum = manifest["install"]["resources"]["min_memory_mb"]
        self.assertLessEqual(minimum, 1024)
        for entry in self.entries:
            for capability in entry["planner_capabilities"]:
                request = capability.get("resource_request", entry["resource_request"])
                self.assertLessEqual(request["memory_mb"], minimum // 2)


if __name__ == "__main__":
    unittest.main()
