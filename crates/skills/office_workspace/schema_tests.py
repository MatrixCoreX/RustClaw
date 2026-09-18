"""Office registry arrays retain matrix and scalar shapes without object wrappers."""
import tomllib
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[3]


class OfficeSchemaTests(unittest.TestCase):
    def schemas(self):
        for relative in ("configs/skills_registry.toml", "docker/config/skills_registry.toml"):
            registry = tomllib.loads((ROOT / relative).read_text())
            yield next(s["input_schema"] for s in registry["skills"] if s["name"] == "office_workspace")

    def test_values_keep_matrix_and_scalar_shapes(self):
        for schema in self.schemas():
            Draft202012Validator.check_schema(schema)
            for values in ([["Name", "Value"], ["alpha", 13]], [1, 2], ["a", "b"]):
                Draft202012Validator(schema).validate({"action": "spreadsheet.create", "operations": [{"op": "set_range", "values": values}]})

    def test_wrapped_rows_and_nested_objects_are_rejected(self):
        for schema in self.schemas():
            for field, bad in (("values", [{"item": ["alpha", 13]}]),
                               ("values", [[{"item": 13}]]),
                               ("rows", [{"item": ["a", "b"]}]),
                               ("rows", ["a", "b"])):
                with self.subTest(field=field, bad=bad):
                    self.assertTrue(list(Draft202012Validator(schema).iter_errors(
                        {"action": "spreadsheet.create", "operations": [{"op": "set_range", field: bad}]})))

    def test_table_rows_are_matrices(self):
        for schema in self.schemas():
            Draft202012Validator(schema).validate({"action": "word.create", "operations": [
                {"op": "add_table", "rows": [["label", "value"], ["ready", 2]]}]})


if __name__ == "__main__":
    unittest.main()
