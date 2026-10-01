#!/usr/bin/env python3
import unittest

from tag_run_outcomes import classify


class TagRunOutcomesTests(unittest.TestCase):
    def test_visible_durable_clarification_is_not_a_runtime_failure(self):
        payload = {
            "data": {
                "status": "running",
                "lifecycle": {
                    "state": "needs_user",
                    "reply_id": "reply-1",
                },
                "result_json": {
                    "messages": [
                        {
                            "reply_id": "reply-1",
                            "relation": "clarification",
                            "text": "Provide the required value.",
                        }
                    ]
                },
            }
        }

        self.assertEqual(
            classify(payload),
            ("pass", "durable needs_user clarification is visible"),
        )

    def test_needs_user_without_visible_reply_remains_a_runtime_failure(self):
        payload = {
            "data": {
                "status": "running",
                "lifecycle": {
                    "state": "needs_user",
                    "reply_id": "reply-1",
                },
                "result_json": {},
            }
        }

        label, reason = classify(payload)
        self.assertEqual(label, "runtime_bug")
        self.assertIn("missing", reason)


if __name__ == "__main__":
    unittest.main()
