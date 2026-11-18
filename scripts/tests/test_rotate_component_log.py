import gzip
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("rotate_component_log", Path(__file__).resolve().parents[1] / "rotate_component_log.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LogRotationTests(unittest.TestCase):
    def test_preserves_bytes_across_restarts_and_bounds_only_own_archives(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "service.log"
            foreign = root / "other.log.previous.1.1.log.gz"
            foreign.write_bytes(b"unrelated")
            for index in range(5):
                content = f"run {index}: error_code=provider_timeout\n".encode()
                log.write_bytes(content)
                module.rotate(log, keep=3)
                self.assertEqual(log.read_bytes(), b"")
                archives = sorted(root.glob("service.log.previous.*.gz"))
                self.assertEqual(gzip.decompress(archives[-1].read_bytes()), content)
                self.assertLessEqual(len(archives), 3)
            self.assertEqual(foreign.read_bytes(), b"unrelated")

    def test_empty_log_and_symlink_cannot_erase_prior_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "service.log"
            module.rotate(log)
            module.rotate(log)
            self.assertFalse(list(root.glob("*.gz")))
            log.unlink()
            target = root / "evidence"
            target.write_text("retained")
            log.symlink_to(target)
            with self.assertRaisesRegex(ValueError, "symlink"):
                module.rotate(log)
            self.assertEqual(target.read_text(), "retained")

    def test_compression_failure_leaves_original_bytes_recoverable(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "service.log"
            log.write_bytes(b"previous failure")
            with patch.object(module.gzip, "GzipFile", side_effect=OSError("disk full")):
                with self.assertRaises(OSError):
                    module.rotate(log)
            self.assertEqual(list(root.glob("*.log"))[0].read_bytes(), b"previous failure")
