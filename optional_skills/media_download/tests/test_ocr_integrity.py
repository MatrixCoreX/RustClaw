from pathlib import Path
import tempfile
import unittest
from unittest import mock

from test_protocol import load_skill_module


class OcrIntegrityTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.skill = load_skill_module()

    def test_numeric_symbol_spacing_is_not_a_number_change(self):
        for source, reviewed in (
            ("3 \u5343; 3\u4e07", "3\u5343; 3 \u4e07"),
            ("\u4e00 \u5341", "\u4e00\u5341"),
            ("\u2163 \u00bd", "\u2163\u00bd"),
            ("\u0661\u0662 kg", "\u0661\u0662kg"),
        ):
            with self.subTest(source=source):
                self.assertTrue(self.skill._revision_preserves_source(source, reviewed))

    def test_distinct_decimal_runs_cannot_be_merged_or_split(self):
        for source, reviewed in (
            ("12 34", "1234"),
            ("1234", "12 34"),
            ("\u0661\u0662 \u0663", "\u0661\u0662\u0663"),
        ):
            with self.subTest(source=source):
                self.assertFalse(self.skill._revision_preserves_source(source, reviewed))

    def test_changed_numeric_values_and_units_are_rejected(self):
        for source, reviewed in (
            ("128.50", "128.05"),
            ("3\u5343", "3\u4e07"),
            ("\u2163", "\u2165"),
            ("\u00bd", "\u00bc"),
            ("12", "\u0661\u0662"),
            ("20260911", "20260912"),
            ("3\u5343", "3"),
            ("12 34", "34 12"),
        ):
            with self.subTest(source=source):
                self.assertFalse(self.skill._revision_preserves_source(source, reviewed))

    def test_empty_or_substantially_truncated_review_is_rejected(self):
        self.assertFalse(self.skill._revision_preserves_source("text", "  "))
        self.assertFalse(self.skill._revision_preserves_source("source " * 20, "source"))

    def test_spacing_review_updates_artifact_and_preserves_raw_backup(self):
        source = "3 \u5343; 10 \u4e2a; 800 \u591a \u4e07"
        reviewed = "3\u5343; 10\u4e2a; 800\u591a\u4e07"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "image_text_ocr.txt"
            path.write_text(source, encoding="utf-8")
            artifacts = [{"path": str(path), "recognition_source": "local_ocr"}]
            with (
                mock.patch.object(self.skill, "_image_text_revision_prompt", return_value="__RAW_RECOGNIZED_TEXT__"),
                mock.patch.object(self.skill, "_internal_llm_revision", return_value=(reviewed, {"status": "reviewed", "reviewed_by_model": True})),
            ):
                metadata = self.skill._review_local_ocr_artifact(artifacts)
            self.assertTrue(metadata["reviewed_by_model"])
            self.assertEqual(path.read_text(encoding="utf-8"), reviewed + "\n")
            self.assertEqual(path.with_name("image_text_ocr_raw.txt").read_text(encoding="utf-8"), source + "\n")


if __name__ == "__main__":
    unittest.main()
