"""Exercise real Tk resources without network, chip access or user settings."""
from contextlib import ExitStack
import gc
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import agent_small_screen as screen
import small_screen_config as config


def resource_counts(root):
    return {
        "images": len(root.tk.splitlist(root.tk.call("image", "names"))),
        "fonts": len(root.tk.splitlist(root.tk.call("font", "names"))),
        "timers": len(root.tk.splitlist(root.tk.call("after", "info"))),
        "commands": len(root.tk.splitlist(root.tk.call("info", "commands"))),
    }


@unittest.skipUnless(os.environ.get("DISPLAY"), "requires an X display")
class SmallScreenUIResourceTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        directory = self.stack.enter_context(tempfile.TemporaryDirectory())
        self.stack.enter_context(mock.patch.object(config, "_settings_file", return_value=str(Path(directory) / "settings.json")))
        replacements = {
            "_single_instance_lock": (None, False),
            "ensure_agent_started": False,
            "load_preferred_runtime_auth_key": "",
            "load_theme": "default",
            "load_lang": "CN",
            "find_splash_image": None,
            "list_gallery_images": [],
            "is_raspberry_pi_device": False,
            "load_login_challenge": None,
            "publish_login_companion_heartbeat": False,
            "_root_dir": directory,
            "_load_skill_data_root_from_config": directory,
        }
        for name, value in replacements.items():
            self.stack.enter_context(mock.patch.object(screen, name, return_value=value))
        self.stack.enter_context(mock.patch.object(screen.threading.Thread, "start"))
        self.app = screen.SmallScreenApp()
        self.addCleanup(self.app._on_close)
        self.app.root.withdraw()

    def refresh_all_views(self):
        for mode in self.app._visible_view_modes():
            self.app._switch_view(mode)
            self.app.root.update_idletasks()
        gc.collect()
        return resource_counts(self.app.root)

    def test_repeated_page_refreshes_plateau_graphical_resources(self):
        self.refresh_all_views()
        samples = [self.refresh_all_views() for _ in range(25)]
        for key, tolerance in (("images", 0), ("fonts", 16), ("timers", 1), ("commands", 8)):
            with self.subTest(resource=key):
                self.assertLessEqual(max(row[key] for row in samples[-10:]) - min(row[key] for row in samples[-10:]), tolerance, samples)

    def test_theme_rebuilds_release_old_images_fonts_and_timers(self):
        self.refresh_all_views()
        samples = []
        for index in range(12):
            self.app._theme = "matrix" if index % 2 else "default"
            self.app._rebuild_ui()
            samples.append(self.refresh_all_views())
        # Compare like-for-like themes after both have warmed their resources.
        for theme_samples in (samples[4::2], samples[5::2]):
            for key, tolerance in (("images", 0), ("fonts", 16), ("timers", 1), ("commands", 8)):
                with self.subTest(resource=key):
                    self.assertLessEqual(max(row[key] for row in theme_samples) - min(row[key] for row in theme_samples), tolerance, samples)


if __name__ == "__main__":
    unittest.main()
