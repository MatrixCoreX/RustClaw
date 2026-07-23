import json
import pathlib
import subprocess
import sys
import unittest


class SignatureProtocolTests(unittest.TestCase):
    def test_pubkey_probe_returns_single_json_line_without_traceback(self):
        script = pathlib.Path(__file__).with_name("signature.py")
        result = subprocess.run(
            [sys.executable, str(script), "pubkey"],
            capture_output=True,
            check=False,
            text=True,
            timeout=15,
        )

        output_lines = [line for line in result.stdout.splitlines() if line.strip()]
        self.assertEqual(len(output_lines), 1, result.stdout)
        payload = json.loads(output_lines[0])
        self.assertIsInstance(payload.get("ok"), bool)
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
