import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).parents[1] / "src" / "tool" / "image_ocr.py"


def load_image_ocr_module():
    spec = importlib.util.spec_from_file_location("media_download_image_ocr", MODULE_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class ImageOcrDocumentTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.image_ocr = load_image_ocr_module()

    def test_multiple_images_form_one_ordered_document_without_source_labels(self) -> None:
        results = [
            self.image_ocr.OcrResult(Path("/private/page-01.jpg"), "第一段\n第一段续行"),
            self.image_ocr.OcrResult(Path("/private/empty.jpg"), "   \n"),
            self.image_ocr.OcrResult(Path("/private/page-03.jpg"), "第二段"),
        ]

        document = self.image_ocr.render_ocr_results(results)

        self.assertEqual(document, "第一段\n第一段续行\n\n第二段\n")
        self.assertNotIn("page-01.jpg", document)
        self.assertNotIn("page-03.jpg", document)
        self.assertNotIn("##", document)

    def test_all_empty_images_produce_an_empty_document(self) -> None:
        results = [
            self.image_ocr.OcrResult(Path("one.jpg"), ""),
            self.image_ocr.OcrResult(Path("two.jpg"), " \n "),
        ]

        self.assertEqual(self.image_ocr.render_ocr_results(results), "")

    def test_short_numeric_lines_are_preserved_when_confidence_is_sufficient(self) -> None:
        tsv = "\n".join(
            [
                "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext",
                "5\t1\t1\t1\t1\t1\t0\t0\t40\t10\t95\t2026",
                "5\t1\t1\t1\t2\t1\t0\t20\t50\t10\t95\t128.50",
            ]
        )

        parsed = self.image_ocr.parse_tesseract_tsv(tsv, min_line_confidence=30)

        self.assertEqual(parsed, "2026\n128.50")

    def test_candidate_score_has_no_script_specific_preference(self) -> None:
        latin = self.image_ocr.ParsedOcrText("abcd", 90.0)
        cjk = self.image_ocr.ParsedOcrText("文字内容", 90.0)

        self.assertEqual(
            self.image_ocr._ocr_candidate_score(latin),
            self.image_ocr._ocr_candidate_score(cjk),
        )

    def test_auto_language_uses_all_installed_recognition_data(self) -> None:
        self.image_ocr.available_tesseract_languages.cache_clear()
        completed = subprocess.CompletedProcess(
            ["tesseract", "--list-langs"],
            0,
            "List of available languages in /tmp/tessdata (4):\neng\nchi_sim\nara\nosd\n",
            "",
        )
        with mock.patch.object(self.image_ocr.subprocess, "run", return_value=completed):
            resolved = self.image_ocr.resolve_tesseract_language("tesseract", "auto")

        self.assertEqual(resolved, "ara+chi_sim+eng")
        self.image_ocr.available_tesseract_languages.cache_clear()


if __name__ == "__main__":
    unittest.main()
