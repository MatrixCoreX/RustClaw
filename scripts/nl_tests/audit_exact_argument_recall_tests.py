import copy
import json
from pathlib import Path
import tempfile
import unittest

from audit_exact_argument_recall import audit


class ExactArgumentAcceptanceTests(unittest.TestCase):
    def fixture(self, root):
        names = ["seed_exact_read", "recall_exact_arguments"]
        (root / "summary.jsonl").write_text("\n".join(json.dumps({"case_name": name, "task_id": str(i),
            "assertion": "pass"}) for i, name in enumerate(names)))
        args = {"action": "read_range", "path": "README.md", "mode": "head", "n": 1}
        observation = {"source_task_id": "0", "instruction_authority": "none",
            "current_execution_evidence": False, "fields": {"/capabilities/0/args": args}}
        streams = [[{"capability": "filesystem.read_text_range", "status": "ok", "provenance": {
            "continuation_args": args, "continuation_args_projection": {"schema_version": 2, "truncated": False}}}],
            [{"capability": "task_control.recall_context", "status": "ok", "data": {"extra": observation}}]]
        paths = []
        for i, name in enumerate(names):
            directory = root / f"case_{i}_{name}"
            directory.mkdir()
            path = directory / "final.json"
            path.write_text(json.dumps({"data": {"status": "succeeded", "result_json": {
                "text": json.dumps(args), "task_journal": {"trace": {"capability_results": streams[i]}}}}}))
            paths.append(path)
        return paths

    def test_complete_actual_argument_observation_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            self.assertEqual(audit(root)["status"], "passed")

    def test_old_gap_wrong_args_replay_scope_and_presentation_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = self.fixture(root)
            original = [json.loads(path.read_text()) for path in paths]
            for kind in ("missing_path", "wrong_args", "replay", "scope", "authority", "presentation"):
                records = copy.deepcopy(original)
                source = records[0]["data"]["result_json"]["task_journal"]["trace"]["capability_results"][0]
                call = records[1]["data"]["result_json"]["task_journal"]["trace"]["capability_results"][0]
                if kind == "missing_path":
                    del source["provenance"]["continuation_args"]["path"]
                elif kind == "wrong_args":
                    call["data"]["extra"]["fields"]["/capabilities/0/args"]["n"] = 2
                elif kind == "replay":
                    call["capability"] = "filesystem.read_text_range"
                elif kind == "scope":
                    call["data"]["extra"]["source_task_id"] = "another-task"
                elif kind == "authority":
                    call["data"]["extra"]["current_execution_evidence"] = True
                else:
                    records[1]["data"]["result_json"]["text"] = '{"mode":"head","n":1}'
                for path, record in zip(paths, records):
                    path.write_text(json.dumps(record))
                with self.subTest(kind=kind), self.assertRaises(ValueError):
                    audit(root)


if __name__ == "__main__":
    unittest.main()
