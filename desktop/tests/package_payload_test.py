"""Keep package identity checking strict while accounting for Tauri's marker."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    'windows_package_payload', Path(__file__).resolve().parents[1] / 'scripts/windows_package_payload.py')
payload = importlib.util.module_from_spec(spec)
spec.loader.exec_module(payload)


class PackagePayloadTest(unittest.TestCase):
    def test_only_first_marker_changes_and_all_other_bytes_are_preserved(self):
        raw = b'MZ\x00binary-data\xff' + payload.PLACEHOLDER + b'\x00suffix' + payload.PLACEHOLDER
        actual, offset = payload.nsis_payload(raw)
        self.assertEqual(len(actual), len(raw))
        self.assertEqual(offset, len(b'MZ\x00binary-data\xff'))
        self.assertEqual(actual[:offset], raw[:offset])
        end = offset + len(payload.PLACEHOLDER)
        self.assertEqual(actual[offset:end], payload.NSIS_MARKER)
        self.assertEqual(actual[end:], raw[end:])
        tampered = actual[:-1] + b'x'
        self.assertNotEqual(tampered, payload.nsis_payload(raw)[0])

    def test_missing_or_already_patched_placeholder_is_rejected(self):
        for raw in [b'MZplain', b'MZ' + payload.NSIS_MARKER]:
            with self.assertRaisesRegex(ValueError, 'tauri_bundle_placeholder_missing'):
                payload.nsis_payload(raw)


if __name__ == '__main__':
    unittest.main()
