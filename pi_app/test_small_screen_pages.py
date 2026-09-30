import unittest
import json
import tempfile
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import agent_small_screen as screen
import small_screen_config as config


class SmallScreenPageVisibilityTests(unittest.TestCase):
    def test_skills_page_is_not_part_of_the_swipe_sequence(self):
        app = SimpleNamespace(
            _show_messages_page=True,
            _show_companion_page=True,  # A legacy setting must not restore the removed page.
            _show_logs_page=True,
            _show_skills_page=True,  # A legacy setting must not restore the removed page.
            _show_weather_page=True,
            _show_stock_page=True,
            _show_us_stock_page=True,
            _show_crypto_page=True,
            _show_bancor_page=True,
            _show_gallery_page=True,
        )

        modes = screen.SmallScreenApp._visible_view_modes(app)

        self.assertNotIn("skills", modes)
        self.assertIn("bancor", modes)
        self.assertNotIn("companion", modes)
        self.assertEqual(modes[0:2], ["dashboard", "overview"])
        self.assertEqual(modes[-1], "settings")

    def test_default_settings_no_longer_expose_a_skills_page_switch(self):
        self.assertNotIn("show_skills", config._default_settings())
        self.assertTrue(config._default_settings()["show_bancor"])
        self.assertNotIn("show_companion", config._default_settings())

    def test_settings_migration_removes_legacy_companion_switch(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            settings_path = Path(temp_dir) / "settings.json"
            settings_path.write_text(
                json.dumps({"show_companion": True}),
                encoding="utf-8",
            )
            with mock.patch.object(config, "_settings_file", return_value=str(settings_path)):
                migrated = config.migrate_small_screen_settings()

            self.assertNotIn("show_companion", migrated)
            self.assertNotIn(
                "show_companion",
                json.loads(settings_path.read_text(encoding="utf-8")),
            )


if __name__ == "__main__":
    unittest.main()
