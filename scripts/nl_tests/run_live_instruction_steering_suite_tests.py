#!/usr/bin/env python3
"""Regression tests for the live instruction-steering harness."""

import json
import unittest

from run_live_instruction_steering_suite import (
    accepted_planner_relations,
    evaluate,
    injection_step_observed,
    task_event_is_terminal,
    task_event_matches_step,
)


def task_with_trace(trace):
    return {"result_json": {"task_journal": {"trace": trace}}}


class LiveInstructionSteeringSuiteTests(unittest.TestCase):
    def test_relations_come_only_from_accepted_journal_rounds(self):
        task = task_with_trace(
            {
                "rounds": [
                    {
                        "decision_envelope": {
                            "conversation_relation": "clarify"
                        }
                    }
                ]
            }
        )

        self.assertEqual(accepted_planner_relations(task), ["clarify"])

    def test_serialized_recovery_rounds_are_supported(self):
        rounds = [
            {"decision_envelope": {"conversation_relation": "amend_current"}},
            {"decision_envelope": {"conversation_relation": "continue_current"}},
        ]
        task = task_with_trace({"rounds": json.dumps(rounds)})

        self.assertEqual(
            accepted_planner_relations(task),
            ["amend_current", "continue_current"],
        )

    def test_top_level_relation_survives_truncated_round_trace(self):
        task = {
            "result_json": {
                "task_journal": {
                    "summary": {
                        "latest_planner_conversation_relation": "amend_current"
                    },
                    "trace": {"rounds": "[{truncated"},
                }
            }
        }

        self.assertEqual(accepted_planner_relations(task), ["amend_current"])

    def test_duplicate_relation_across_trace_and_round_is_reported_once(self):
        task = task_with_trace(
            {
                "latest_planner_conversation_relation": "clarify",
                "rounds": [
                    {
                        "decision_envelope": {
                            "conversation_relation": "clarify"
                        }
                    }
                ],
            }
        )

        self.assertEqual(accepted_planner_relations(task), ["clarify"])

    def test_exact_action_gate_does_not_match_an_earlier_skill_action(self):
        mkdir_only = task_with_trace(
            {
                "step_results": [
                    {
                        "status": "ok",
                        "skill": "fs_basic",
                        "requested_action_ref": "filesystem.make_dir",
                    }
                ]
            }
        )
        after_write = task_with_trace(
            {
                "step_results": [
                    *mkdir_only["result_json"]["task_journal"]["trace"]["step_results"],
                    {
                        "status": "ok",
                        "skill": "fs_basic",
                        "requested_action_ref": "filesystem.write_file",
                    },
                ]
            }
        )

        self.assertFalse(
            injection_step_observed(
                mkdir_only, set(), {"filesystem.write_file"}
            )
        )
        self.assertTrue(
            injection_step_observed(
                after_write, {"fs_basic"}, {"filesystem.write_file"}
            )
        )

    def test_sse_barrier_matches_finished_exact_action_only(self):
        mkdir = {
            "event_kind": "tool_finished",
            "payload": {
                "status": "ok",
                "skill": "fs_basic",
                "requested_action_ref": "filesystem.make_dir",
            },
        }
        write = {
            "event_kind": "tool_finished",
            "payload": {
                "status": "ok",
                "skill": "fs_basic",
                "requested_action_ref": "filesystem.write_file",
            },
        }

        self.assertFalse(
            task_event_matches_step(mkdir, set(), {"filesystem.write_file"})
        )
        self.assertTrue(
            task_event_matches_step(
                write, {"fs_basic"}, {"filesystem.write_file"}
            )
        )
        self.assertFalse(task_event_is_terminal(write))
        self.assertTrue(task_event_is_terminal({"event_kind": "task_final"}))

    def test_semantic_output_contract_checks_json_keys_and_minimum_lines(self):
        task = {
            "status": "succeeded",
            "result_json": {"text": '{"summary":"one\\ntwo","marker":"done"}'},
        }
        checks = evaluate(
            {
                "expected": {
                    "json_object_keys": ["summary", "marker"],
                    "min_nonblank_lines": 1,
                }
            },
            task,
            [],
            [],
            [],
            [],
            [],
            [],
        )

        self.assertTrue(checks["json_object_keys"])
        self.assertTrue(checks["min_nonblank_lines"])

    def test_json_contract_rejects_extra_keys_and_non_json(self):
        for text in (
            '{"summary":"one","marker":"done","extra":true}',
            "```json\\n{\\\"summary\\\":\\\"one\\\",\\\"marker\\\":\\\"done\\\"}\\n```",
        ):
            task = {"status": "succeeded", "result_json": {"text": text}}
            checks = evaluate(
                {"expected": {"json_object_keys": ["summary", "marker"]}},
                task,
                [],
                [],
                [],
                [],
                [],
                [],
            )

            self.assertFalse(checks["json_object_keys"])

    def test_accepted_outcomes_allow_success_blocker_or_waiting(self):
        case = {
            "expected": {
                "accepted_outcomes": [
                    {
                        "status": "succeeded",
                        "contains": ["PROBE-01"],
                        "max_llm_calls": 6,
                    },
                    {
                        "status": "succeeded",
                        "contains": ["resource_admission_unavailable"],
                        "absent": ["PROBE-01"],
                        "max_llm_calls": 8,
                    },
                    {
                        "status": "running",
                        "lifecycle_state": "waiting",
                        "max_llm_calls": 6,
                    },
                ]
            }
        }
        success = {
            "status": "succeeded",
            "result_json": {"text": "PROBE-01"},
        }
        blocker = {
            "status": "succeeded",
            "result_json": {"text": "resource_admission_unavailable"},
        }
        waiting = {
            "status": "running",
            "lifecycle": {"state": "waiting"},
            "result_json": {"text": ""},
        }

        for task, call_count in ((success, 6), (blocker, 7), (waiting, 4)):
            checks = evaluate(
                case,
                task,
                [],
                [],
                [{} for _ in range(call_count)],
                [],
                [],
                [],
            )
            self.assertTrue(checks["terminal_status"])
            self.assertTrue(checks["lifecycle_state"])
            self.assertTrue(checks["accepted_outcome"])


if __name__ == "__main__":
    unittest.main()
