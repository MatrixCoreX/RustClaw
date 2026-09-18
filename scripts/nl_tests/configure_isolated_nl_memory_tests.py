#!/usr/bin/env python3
"""Memory test setup must refuse production roots and non-loopback endpoints."""
import contextlib
import io
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from configure_isolated_nl_memory import main


class IsolatedMemorySettingsTests(unittest.TestCase):
    def test_rejects_non_test_root_before_reading_authentication(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch("sys.argv", ["test", "--isolation-root", directory, "--base-url", "http://127.0.0.1:1"]):
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                    main()

    def test_rejects_remote_endpoint_before_reading_authentication(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tmp/agent-runtime-nl-isolated-fixture"
            root.mkdir(parents=True)
            for endpoint in ("http://example.invalid:1", "http://127.0.0.1", "https://127.0.0.1:1"):
                with self.subTest(endpoint=endpoint), patch("sys.argv", ["test", "--isolation-root", str(root), "--base-url", endpoint]):
                    with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                        main()


if __name__ == "__main__":
    unittest.main()
