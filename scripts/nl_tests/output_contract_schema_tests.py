"""Keep semantic presentation labels separate from exact source projections."""
from copy import deepcopy
import json
from pathlib import Path
import unittest

import jsonschema

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = json.loads((ROOT / "prompts/schemas/plan_result.schema.json").read_text())["$defs"]["OutputContract"]
BASE = {
    "response_shape": "strict", "exact_sentence_count": None,
    "requires_content_evidence": True, "delivery_required": False,
    "locator_kind": "path", "delivery_intent": "none",
    "structured_field_selector": None, "output_field_labels": ["new_count", "read_count", "play_count"],
}


class OutputContractSchemaTests(unittest.TestCase):
    def test_semantic_labels_are_valid(self):
        jsonschema.validate(BASE, SCHEMA)
        jsonschema.validate(dict(BASE, output_field_labels=["文字", "text", "entry.path"]), SCHEMA)

    def test_labels_cannot_be_mixed_with_source_selectors(self):
        invalid = deepcopy(BASE)
        invalid["structured_field_selector"] = "run.new_records"
        with self.assertRaises(jsonschema.ValidationError):
            jsonschema.validate(invalid, SCHEMA)

    def test_semantic_labels_require_strict_shape(self):
        for shape in ("free", "scalar", "file_token", "one_sentence"):
            invalid = dict(BASE, response_shape=shape)
            with self.assertRaises(jsonschema.ValidationError):
                jsonschema.validate(invalid, SCHEMA)

    def test_existing_source_contracts_remain_valid(self):
        existing = dict(BASE, structured_field_selector="count")
        del existing["output_field_labels"]
        jsonschema.validate(existing, SCHEMA)
        existing["output_field_labels"] = None
        jsonschema.validate(existing, SCHEMA)


if __name__ == "__main__":
    unittest.main()
