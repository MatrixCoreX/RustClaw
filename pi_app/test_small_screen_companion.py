import tempfile
import unittest
from pathlib import Path
from unittest import mock

import small_screen_config as config
from small_screen_companion import compact_companion_text, select_companion_message


class SmallScreenCompanionTests(unittest.TestCase):
    def test_selects_latest_channel_question_while_waiting(self):
        state = select_companion_message(
            [
                {
                    "task_id": "task-new",
                    "time": "12:00:00",
                    "channel": "wechat",
                    "question": "帮我查看天气",
                    "reply": "",
                },
                {
                    "task_id": "task-old",
                    "question": "旧消息",
                    "reply": "旧回复",
                },
            ]
        )

        self.assertEqual(state.task_id, "task-new")
        self.assertEqual(state.channel, "wechat")
        self.assertTrue(state.is_waiting)
        self.assertFalse(state.is_replying)

    def test_stops_for_the_same_reply_delivered_to_the_channel(self):
        state = select_companion_message(
            [
                {
                    "task_id": "task-1",
                    "channel": "telegram",
                    "question": "总结这段内容",
                    "reply": "已经整理完成。",
                }
            ]
        )

        self.assertTrue(state.is_replying)
        self.assertFalse(state.is_waiting)
        self.assertEqual(state.reply, "已经整理完成。")

    def test_ignores_invalid_items_and_normalizes_multiline_text(self):
        state = select_companion_message(
            [None, {}, {"channel": "ui", "question": "第一行\n 第二行"}]
        )

        self.assertEqual(state.question, "第一行 第二行")
        self.assertTrue(state.is_waiting)

    def test_compacts_long_text_for_the_small_screen(self):
        self.assertEqual(compact_companion_text("abcdef", 5), "abcd…")
        self.assertEqual(compact_companion_text("abc", 5), "abc")

    def test_companion_visibility_is_persisted_and_migrated(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            settings_path = Path(temp_dir) / "settings.json"
            with mock.patch.object(config, "_settings_file", return_value=str(settings_path)):
                self.assertTrue(config.load_companion_page_visible())
                config.save_companion_page_visible(False)
                self.assertFalse(config.load_companion_page_visible())
                migrated = config.migrate_small_screen_settings()
                self.assertFalse(migrated["show_companion"])


if __name__ == "__main__":
    unittest.main()
