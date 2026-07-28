import importlib.util
from pathlib import Path
import unittest


class ProtocolTest(unittest.TestCase):
    def test_response_echoes_request_id(self) -> None:
        path = Path(__file__).parents[1] / "src" / "main.py"
        spec = importlib.util.spec_from_file_location("skill_main", path)
        module = importlib.util.module_from_spec(spec)
        assert spec and spec.loader
        spec.loader.exec_module(module)
        self.assertEqual(module.respond({"request_id": "test-1"})["request_id"], "test-1")


if __name__ == "__main__":
    unittest.main()
