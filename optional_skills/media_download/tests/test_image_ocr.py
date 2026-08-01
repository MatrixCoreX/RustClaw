import importlib.util
from pathlib import Path
import sys
import unittest


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


if __name__ == "__main__":
    unittest.main()
