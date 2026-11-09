import gc
import os
from types import SimpleNamespace
import unittest
from unittest import mock
import tkinter as tk
import tkinter.font as tkfont

import small_screen_font_cache as fonts
from small_screen_formatters import _line_clamp_text


class MeasurementFontCacheTests(unittest.TestCase):
    def setUp(self):
        self.root = SimpleNamespace(bind=mock.Mock())

    def test_repeated_refreshes_reuse_one_font_and_one_cleanup_hook(self):
        with mock.patch.object(fonts.tkfont, "Font") as factory:
            original = fonts.measurement_font(("", 11), self.root)
            for _ in range(2000):
                self.assertIs(fonts.measurement_font(["", 11], self.root), original)
        self.assertEqual(factory.call_count, 1)
        self.root.bind.assert_called_once()

    def test_lru_cache_is_bounded_and_eviction_drops_font_reference(self):
        with mock.patch.object(fonts.tkfont, "Font", side_effect=lambda **kw: object()) as factory:
            first = fonts.measurement_font(("", 10), self.root)
            for size in range(11, 11 + fonts.MAX_MEASUREMENT_FONTS):
                fonts.measurement_font(("", size), self.root)
            self.assertIsNot(fonts.measurement_font(("", 10), self.root), first)
        self.assertEqual(len(getattr(self.root, fonts._CACHE_ATTRIBUTE)), fonts.MAX_MEASUREMENT_FONTS)
        self.assertEqual(factory.call_count, fonts.MAX_MEASUREMENT_FONTS + 2)

    def test_cache_does_not_cross_tk_interpreters(self):
        other = SimpleNamespace(bind=mock.Mock())
        with mock.patch.object(fonts.tkfont, "Font", side_effect=lambda **kw: object()):
            self.assertIsNot(fonts.measurement_font(("", 11), self.root), fonts.measurement_font(("", 11), other))

    def test_rebuild_clears_fonts_without_registering_another_root_callback(self):
        with mock.patch.object(fonts.tkfont, "Font", side_effect=lambda **kw: object()):
            first = fonts.measurement_font(("", 11), self.root)
            fonts.clear_measurement_fonts(self.root)
            self.assertIsNot(fonts.measurement_font(("", 11), self.root), first)
        self.root.bind.assert_called_once()

    def test_child_destroy_keeps_fonts_but_root_destroy_releases_them(self):
        with mock.patch.object(fonts.tkfont, "Font"):
            fonts.measurement_font(("", 11), self.root)
        cleanup = self.root.bind.call_args.args[1]
        cleanup(SimpleNamespace(widget=object()))
        self.assertEqual(len(getattr(self.root, fonts._CACHE_ATTRIBUTE)), 1)
        cleanup(SimpleNamespace(widget=self.root))
        self.assertFalse(getattr(self.root, fonts._CACHE_ATTRIBUTE))

    def test_named_and_mutable_font_descriptors_are_not_cached(self):
        with mock.patch.object(fonts.tkfont, "Font") as factory:
            for _ in range(2):
                fonts.measurement_font("TkDefaultFont", self.root)
                fonts.measurement_font(object(), self.root)
        self.assertEqual(factory.call_count, 4)
        self.root.bind.assert_not_called()

    def test_empty_or_unclamped_text_does_not_allocate_a_font(self):
        with mock.patch("small_screen_formatters.measurement_font") as factory:
            self.assertEqual(_line_clamp_text("", ("", 11), 40), "\n\n")
            self.assertEqual(_line_clamp_text("hello", ("", 11), 40, max_lines=0), "hello")
        factory.assert_not_called()


@unittest.skipUnless(os.environ.get("DISPLAY"), "requires an X display")
class TkMeasurementFontTests(unittest.TestCase):
    def setUp(self):
        self.root = tk.Tk()
        self.root.withdraw()

    def tearDown(self):
        self.root.destroy()

    def test_layout_matches_uncached_font_metrics(self):
        samples = ("hello world", "\n", "A\n\nB", "中文排版与标点，完整保留。", "e\u0301👨‍👩‍👧‍👦", "long text " * 80)
        for descriptor in (("", 11), ("", 12, "bold"), ("DejaVu Sans Mono", 9)):
            for text in samples:
                for width in (0, 30, 120):
                    with self.subTest(font=descriptor, text=text[:12], width=width):
                        cached = _line_clamp_text(text, descriptor, width, 2)
                        with mock.patch("small_screen_formatters.measurement_font", side_effect=lambda font: tkfont.Font(root=self.root, font=font)):
                            uncached = _line_clamp_text(text, descriptor, width, 2)
                        self.assertEqual(cached, uncached)

    def test_refresh_font_count_plateaus_and_clear_releases_tcl_fonts(self):
        baseline = set(self.root.tk.splitlist(self.root.tk.call("font", "names")))
        for index in range(2000):
            _line_clamp_text(f"设备在线 {index} — status", ("", 11), 120, 2)
        after = set(self.root.tk.splitlist(self.root.tk.call("font", "names")))
        self.assertEqual(len(after - baseline), 1)
        fonts.clear_measurement_fonts(self.root)
        gc.collect()
        self.assertEqual(set(self.root.tk.splitlist(self.root.tk.call("font", "names"))), baseline)


if __name__ == "__main__":
    unittest.main()
