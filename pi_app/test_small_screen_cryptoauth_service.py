import unittest
from unittest import mock

import small_screen_cryptoauth_service as cryptoauth


class LocalSignatureChipTestTests(unittest.TestCase):
    def test_chip_test_reads_and_signs_locally_without_nni_client(self):
        with mock.patch.object(
            cryptoauth,
            "_run_signature_helper",
            side_effect=[
                ({"ok": True, "pubkey": "ab" * 64}, ""),
                ({"ok": True, "signature": "cd" * 64}, ""),
            ],
        ) as helper:
            result, error = cryptoauth.test_signature_chip_via_helper()

        self.assertEqual(error, "")
        self.assertEqual(result["pubkey"], "ab" * 64)
        self.assertEqual(result["signature"], "cd" * 64)
        self.assertEqual(helper.call_args_list[0], mock.call(["pubkey"]))
        action, challenge = helper.call_args_list[1].args[0]
        self.assertEqual(action, "sign_challenge")
        self.assertEqual(len(challenge), 64)

    def test_chip_test_stops_before_signing_when_pubkey_read_fails(self):
        with mock.patch.object(
            cryptoauth,
            "_run_signature_helper",
            return_value=(None, "chip unavailable"),
        ) as helper:
            result, error = cryptoauth.test_signature_chip_via_helper()

        self.assertIsNone(result)
        self.assertEqual(error, "chip unavailable")
        helper.assert_called_once_with(["pubkey"])


if __name__ == "__main__":
    unittest.main()
