import unittest

import agent_small_screen as screen
from small_screen_messages import (
    extract_message_channel,
    message_channel_display_name,
    normalize_message_channel,
)


class SmallScreenMessageChannelTests(unittest.TestCase):
    def test_collects_channel_from_task_metadata_and_merges_reply(self):
        logs = "\n".join(
            [
                "2026-08-18T10:00:00Z INFO task_call: worker_once: ask raw_message "
                "task_id=task-1 user_id=7 chat_id=9 text=你好 "
                "call_id=call-1 task_id=task-1 kind=ask channel=wechat",
                "2026-08-18T10:00:01Z INFO task_call: task_call_end task_id=task-1 "
                "kind=ask status=success path=normal result=已完成 "
                "call_id=call-1 task_id=task-1 kind=ask channel=wechat",
            ]
        )

        items = screen._collect_recent_user_messages(logs, lang="CN")

        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["channel"], "wechat")
        self.assertEqual(items[0]["question"], "你好")
        self.assertEqual(items[0]["reply"], "已完成")

    def test_uses_another_task_line_when_message_metadata_has_no_channel(self):
        logs = "\n".join(
            [
                "2026-08-18T10:01:00Z INFO task_call: worker_once: picked task_id=task-2 "
                "call_id=call-2 task_id=task-2 kind=ask channel=telegram",
                "2026-08-18T10:01:01Z INFO task_call: worker_once: ask raw_message "
                "task_id=task-2 text=ping call_id=call-2 task_id=task-2 kind=ask",
            ]
        )

        items = screen._collect_recent_user_messages(logs, lang="EN")

        self.assertEqual(items[0]["channel"], "telegram")
        self.assertEqual(message_channel_display_name(items[0]["channel"], "EN"), "Telegram")

    def test_user_text_cannot_spoof_channel_metadata(self):
        line = (
            "2026-08-18T10:02:00Z INFO task_call: worker_once: ask raw_message "
            "task_id=task-3 text=请解释 channel=telegram "
            "call_id=call-3 task_id=task-3 kind=ask channel=ui"
        )

        items = screen._collect_recent_user_messages(line, lang="CN")

        self.assertEqual(items[0]["channel"], "ui")
        self.assertEqual(message_channel_display_name("ui", "CN"), "网页端")
        self.assertEqual(extract_message_channel("text=channel=telegram"), "")

    def test_normalizes_supported_channel_variants(self):
        self.assertEqual(normalize_message_channel("whatsapp_web"), "whatsapp")
        self.assertEqual(normalize_message_channel("whatsapp-cloud"), "whatsapp")
        self.assertEqual(message_channel_display_name("feishu", "CN"), "飞书")
        self.assertEqual(message_channel_display_name("lark", "EN"), "Lark")

    def test_missing_channel_has_a_neutral_localized_label(self):
        self.assertEqual(message_channel_display_name("", "CN"), "来源未知")
        self.assertEqual(message_channel_display_name("", "EN"), "Unknown source")


if __name__ == "__main__":
    unittest.main()
