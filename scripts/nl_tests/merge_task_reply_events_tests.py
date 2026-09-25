import unittest

from merge_task_reply_events import parse_sse_data, project_reply


class MergeTaskReplyEventsTests(unittest.TestCase):
    def test_projects_matching_clarification(self):
        snapshot = {
            "data": {
                "status": "running",
                "lifecycle": {"state": "needs_user", "reply_id": "reply-2"},
                "result_json": {"text": ""},
            }
        }
        events = parse_sse_data(
            'event: conversation_reply_item\n'
            'data: {"seq":2,"event_kind":"conversation_reply_item","payload":'
            '{"reply_id":"reply-2","relation":"clarification",'
            '"lifecycle_stage":"accepted","text":"Which path?"}}\n\n'
        )

        self.assertTrue(project_reply(snapshot, events))
        self.assertEqual(snapshot["data"]["result_json"]["text"], "Which path?")
        self.assertEqual(
            snapshot["data"]["result_json"]["messages"][0]["reply_id"],
            "reply-2",
        )

    def test_does_not_project_an_unrelated_reply(self):
        snapshot = {
            "data": {
                "lifecycle": {"reply_id": "reply-2"},
                "result_json": {"text": ""},
            }
        }
        events = [
            {
                "seq": 1,
                "event_kind": "conversation_reply_item",
                "payload": {"reply_id": "reply-1", "text": "unrelated"},
            }
        ]

        self.assertFalse(project_reply(snapshot, events))
        self.assertEqual(snapshot["data"]["result_json"]["text"], "")


if __name__ == "__main__":
    unittest.main()
