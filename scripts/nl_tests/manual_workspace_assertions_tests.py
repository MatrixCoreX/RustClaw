"""File lifecycle acceptance follows executed evidence, including local repair."""
import copy
import hashlib
import json
import unittest

from manual_case_assertions import evaluate_expectations


PATH = "tmp/fixture.txt"
CONTENT = b"first\nsecond\n"
DIGEST = hashlib.sha256(CONTENT).hexdigest()
SPEC = {"schema_version": 1, "path": PATH, "sha256": DIGEST,
        "size_bytes": len(CONTENT), "line_count": 2}


def snapshot(content=None):
    return {"path": PATH, "kind": "missing" if content is None else "file",
            "sha256": None if content is None else "sha256:" + hashlib.sha256(content).hexdigest(),
            "size_bytes": None if content is None else len(content)}


def mutation(before, after):
    return {"state": "applied", "status": "ok", "target_path": PATH,
            "before": [snapshot(before)], "after": [snapshot(after)], "changed_files": [PATH]}


def read(content):
    return {"path": PATH, "sha256": hashlib.sha256(content).hexdigest(), "size_bytes": len(content),
            "total_lines": 2, "returned_line_count": 2, "start_line": 1, "end_line": 2,
            "truncated": False, "line_safety": {"excerpt_truncated": False, "truncated_lines": 0}}


def evidence(repair=False):
    initial = CONTENT[:-1] if repair else CONTENT
    operations = [("filesystem.write_file", "mutate", mutation(None, initial)),
                  ("filesystem.read_text_range", "observe", read(initial))]
    if repair:
        operations += [("workspace.replace_text", "mutate", mutation(initial, CONTENT)),
                       ("filesystem.read_text_range", "observe", read(CONTENT))]
    operations += [("filesystem.remove_path", "mutate", mutation(CONTENT, None))]
    steps, caps = [], []
    for i, (capability, effect, output) in enumerate(operations, 1):
        sid = f"step_{i}"
        steps.append({"step_id": sid, "requested_action_type": "call_capability", "status": "ok",
                      "resolved_capability": capability, "executed_skill": "fixture"})
        caps.append({"capability": capability, "effect": effect, "status": "ok", "truncated": False,
                     "provenance": {"source": "runtime_step", "step_id": sid}, "data": {"output": output}})
    return {"text": "", "task_journal": {"trace": {"step_results": steps, "capability_results": caps}}}


def check(result, spec=SPEC):
    return evaluate_expectations("workspace_file_cycle:" + json.dumps(spec), "requires_tool_call=true",
                                 {}, "succeeded", result["text"], result, {})[0]


class WorkspaceLifecycleTests(unittest.TestCase):
    def empty_evidence(self):
        result = evidence()
        caps = result["task_journal"]["trace"]["capability_results"]
        caps[0]["data"]["output"] = mutation(None, b"")
        caps[1]["data"]["output"] = {**read(b""), "total_lines": 0, "returned_line_count": 0,
                                      "start_line": 0, "end_line": 0}
        caps[2]["data"]["output"] = mutation(b"", None)
        spec = {**SPEC, "sha256": hashlib.sha256(b"").hexdigest(), "size_bytes": 0, "line_count": 0}
        return result, spec

    def test_empty_file_uses_zero_line_range(self):
        result, spec = self.empty_evidence()
        self.assertEqual(check(result, spec), "pass")

    def test_empty_file_still_requires_exact_complete_readback(self):
        for field, value in (("start_line", 1), ("end_line", 1), ("total_lines", 1),
                             ("returned_line_count", 1), ("truncated", True),
                             ("sha256", "0" * 64), ("size_bytes", 1)):
            with self.subTest(field=field):
                result, spec = self.empty_evidence()
                result["task_journal"]["trace"]["capability_results"][1]["data"]["output"][field] = value
                self.assertEqual(check(result, spec), "fail")

    def test_empty_file_requires_readback_and_removal(self):
        for index in (1, 2):
            result, spec = self.empty_evidence()
            del result["task_journal"]["trace"]["capability_results"][index]
            self.assertEqual(check(result, spec), "fail")

    def test_exact_write_read_remove(self):
        self.assertEqual(check(evidence()), "pass")

    def test_local_edit_repair_then_exact_read_remove(self):
        self.assertEqual(check(evidence(True)), "pass")

    def test_readback_absolute_path_must_match_runtime_write_locator(self):
        result = evidence()
        caps = result["task_journal"]["trace"]["capability_results"]
        absolute = "/isolated/workspace/" + PATH
        caps[0]["evidence"] = [{"id": "step_1", "source": "filesystem.write_file", "locator": absolute,
                                "metadata": {"step_id": "step_1"}}]
        caps[1]["data"]["output"]["path"] = absolute
        self.assertEqual(check(result), "pass")
        caps[1]["data"]["output"]["path"] = "/other/workspace/" + PATH
        self.assertEqual(check(result), "fail")

    def test_proven_no_op_does_not_mutate_lifecycle(self):
        result = evidence()
        trace = result["task_journal"]["trace"]
        step = {"step_id": "step_0", "requested_action_type": "call_capability", "status": "ok",
                "resolved_capability": "filesystem.make_dir", "executed_skill": "fixture"}
        output = {"state": "no_op", "status": "ok", "target_path": "tmp", "changed_files": [],
                  "before": [{"path": "tmp", "kind": "directory"}], "after": [{"path": "tmp", "kind": "directory"}]}
        cap = {"capability": "filesystem.make_dir", "effect": "mutate", "status": "ok",
               "provenance": {"source": "runtime_step", "step_id": "step_0"}, "data": {"output": output}}
        trace["step_results"].insert(0, step)
        trace["capability_results"].insert(0, cap)
        self.assertEqual(check(result), "pass")
        output["after"][0]["kind"] = "missing"
        self.assertEqual(check(result), "fail")

    def test_same_size_wrong_content_is_not_success(self):
        result = evidence()
        for cap in result["task_journal"]["trace"]["capability_results"]:
            cap["data"]["output"] = json.loads(json.dumps(cap["data"]["output"]).replace(DIGEST, "0" * 64))
        self.assertEqual(check(result), "fail")

    def test_no_readback_or_no_removal(self):
        for index in (1, 2):
            result = evidence()
            del result["task_journal"]["trace"]["capability_results"][index]
            self.assertEqual(check(result), "fail")

    def test_unexecuted_or_failed_evidence(self):
        for field, value in (("requested_action_type", "respond"), ("status", "error")):
            result = evidence()
            result["task_journal"]["trace"]["step_results"][0][field] = value
            self.assertEqual(check(result), "fail")

    def test_wrong_path_cannot_satisfy_expected_content(self):
        result = json.loads(json.dumps(evidence()).replace(PATH, "tmp/another.txt"))
        self.assertEqual(check(result), "fail")

    def test_wrong_mutation_before_digest(self):
        result = evidence(True)
        result["task_journal"]["trace"]["capability_results"][2]["data"]["output"]["before"][0]["sha256"] = "sha256:" + "f" * 64
        self.assertEqual(check(result), "fail")

    def test_repair_requires_fresh_readback(self):
        result = evidence(True)
        del result["task_journal"]["trace"]["capability_results"][3]
        self.assertEqual(check(result), "fail")

    def test_out_of_order_or_duplicate_evidence(self):
        for duplicate in (True, False):
            result = evidence()
            caps = result["task_journal"]["trace"]["capability_results"]
            if duplicate:
                caps.insert(1, copy.deepcopy(caps[0]))
            else:
                caps[1], caps[2] = caps[2], caps[1]
            self.assertEqual(check(result), "fail")

    def test_preview_is_not_a_write(self):
        result = evidence()
        result["task_journal"]["trace"]["capability_results"][0]["data"]["output"]["state"] = "preview"
        self.assertEqual(check(result), "fail")

    def test_partial_or_truncated_readback(self):
        for field, value in (("returned_line_count", 1), ("truncated", True), ("start_line", 2)):
            result = evidence()
            result["task_journal"]["trace"]["capability_results"][1]["data"]["output"][field] = value
            self.assertEqual(check(result), "fail")

    def test_recreated_file_not_cleaned(self):
        result = evidence()
        trace = result["task_journal"]["trace"]
        step, cap = copy.deepcopy(trace["step_results"][0]), copy.deepcopy(trace["capability_results"][0])
        step["step_id"] = cap["provenance"]["step_id"] = "step_4"
        trace["step_results"].append(step)
        trace["capability_results"].append(cap)
        self.assertEqual(check(result), "fail")

    def test_prose_cannot_fake_evidence(self):
        result = {"text": "workspace_file_cycle:" + json.dumps(SPEC)}
        self.assertEqual(check(result), "fail")

    def test_invalid_spec(self):
        for spec in ({}, {**SPEC, "sha256": "bad"}, {**SPEC, "size_bytes": True},
                     {**SPEC, "path": "../outside"}, {**SPEC, "schema_version": 2}):
            self.assertEqual(check(evidence(), spec), "fail")


if __name__ == "__main__":
    unittest.main()
