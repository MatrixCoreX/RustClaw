"""Registry schemas must preserve JSON values and reject malformed operation fields."""
import copy
import tomllib
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[3]


class TransformSchemaTests(unittest.TestCase):
    def test_json_text_is_admitted_by_every_capability_alias(self):
        for relative in ("configs/skills_registry.toml", "docker/config/skills_registry.toml"):
            registry = tomllib.loads((ROOT / relative).read_text())
            skill = next(skill for skill in registry["skills"] if skill["name"] == "transform")
            for capability in skill["planner_capabilities"]:
                with self.subTest(registry=relative, capability=capability["name"]):
                    self.assertIn("json_text", capability["required"][0].split("|"))
        manifest = tomllib.loads((ROOT / "crates/skills/transform/skill.toml").read_text())
        for capability in manifest["capability_request"]["capabilities"]:
            with self.subTest(manifest=True, capability=capability["name"]):
                self.assertIn("json_text", capability["required"][0].split("|"))

    def schemas(self):
        for relative in ("configs/skills_registry.toml", "docker/config/skills_registry.toml"):
            registry = tomllib.loads((ROOT / relative).read_text())
            yield next(skill["input_schema"] for skill in registry["skills"] if skill["name"] == "transform")
        manifest = tomllib.loads((ROOT / "crates/skills/transform/skill.toml").read_text())
        yield manifest["capability_request"]["input_schema"]

    def test_scalar_array_and_record_operations_are_valid(self):
        requests = [
            {"json_text": '[{"count":4,"code":"04"}]', "ops": []},
            {"data": ["a", "b", "a"], "ops": [{"op": "dedup"}]},
            {"data": [{"kind": "x", "n": 2}], "ops": [
                {"op": "filter", "field": "n", "cmp": "gte", "value": 1},
                {"op": "sort", "by": "n", "order": "desc"},
                {"op": "project", "fields": ["kind", "n"]}]},
            {"csv_text": "kind,n\nx,2", "ops": [{"op": "group", "by": ["kind"],
                "aggregations": [{"op": "sum", "field": "n", "name": "total"}]}]},
            {"data": {"old": 2}, "ops": [{"op": "rename", "mappings": [{"from": "old", "to": "new"}]}]},
        ]
        for schema in self.schemas():
            Draft202012Validator.check_schema(schema)
            for request in requests:
                Draft202012Validator(schema).validate(request)

    def test_json_text_requires_nonempty_string(self):
        for schema in self.schemas():
            for value in (None, [], {}, 4, True, ""):
                with self.subTest(value=value):
                    self.assertTrue(list(Draft202012Validator(schema).iter_errors({"json_text": value})))

    def test_operation_fields_reject_object_wrappers_and_wrong_types(self):
        valid = {"data": [{"v": "a"}], "ops": [{"op": "project", "fields": ["v"]}]}
        for schema in self.schemas():
            for field, bad_value in (("fields", {"item": "v"}), ("fields", [3]),
                                     ("mappings", {"from": "v", "to": "text"}),
                                     ("aggregations", "sum"), ("field", ["v"])):
                request = copy.deepcopy(valid)
                request["ops"][0][field] = bad_value
                with self.subTest(field=field, value=bad_value):
                    self.assertTrue(list(Draft202012Validator(schema).iter_errors(request)))


if __name__ == "__main__":
    unittest.main()
