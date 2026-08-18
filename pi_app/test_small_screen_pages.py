import unittest
from types import SimpleNamespace

import agent_small_screen as screen
import small_screen_config as config


class SmallScreenPageVisibilityTests(unittest.TestCase):
    def test_skills_page_is_not_part_of_the_swipe_sequence(self):
        app = SimpleNamespace(
            _show_messages_page=True,
            _show_logs_page=True,
            _show_skills_page=True,  # A legacy setting must not restore the removed page.
            _show_weather_page=True,
            _show_stock_page=True,
            _show_us_stock_page=True,
            _show_crypto_page=True,
            _show_gallery_page=True,
        )

        modes = screen.SmallScreenApp._visible_view_modes(app)

        self.assertNotIn("skills", modes)
        self.assertEqual(modes[0:2], ["dashboard", "overview"])
        self.assertEqual(modes[-1], "settings")

    def test_default_settings_no_longer_expose_a_skills_page_switch(self):
        self.assertNotIn("show_skills", config._default_settings())


if __name__ == "__main__":
    unittest.main()
