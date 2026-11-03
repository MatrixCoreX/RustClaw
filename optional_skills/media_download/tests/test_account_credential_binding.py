"""Identity refresh is credential-bound without disclosing private digests."""
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import test_account_sessions as fixtures
from account_sessions import account_status, public_snapshot, write_snapshot


class CredentialBindingTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.AccountSessionTest()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.profile = self.fixture.profile()

    def verified(self):
        write_snapshot(self.root, {"platform": "douyin", "status": "verified",
                                  "display_name": "Fixture former identity", "checked_at": 1})
        self.assertEqual(account_status(self.root, ["douyin"])[0]["status"], "verified")

    def test_unchanged_credentials_keep_observed_identity_without_extending_its_time(self):
        self.verified()
        file = self.root / "accounts/douyin.json"
        before = file.read_bytes()
        row = account_status(self.root, ["douyin"])[0]
        self.assertEqual(row["display_name"], "Fixture former identity")
        self.assertEqual(row["checked_at"], 1)
        self.assertEqual(file.read_bytes(), before, "refresh is read-only")
        self.assertEqual(os.stat(file).st_mode & 0o777, 0o600)
        for result in (row, public_snapshot(json.loads(before), "douyin")):
            self.assertNotIn("credential_binding", result)
            self.assertNotIn("fixture-cookie-value", json.dumps(result))

    def test_same_length_credential_replacement_invalidates_identity_not_the_saved_session(self):
        for column, replacement in (("value", "fixture-cookie-other"), ("encrypted_value", b"fixture-ciphertext")):
            with self.subTest(column=column):
                self.verified()
                with closing(sqlite3.connect(self.profile / "Default/Network/Cookies")) as database:
                    database.execute(f"UPDATE cookies SET {column}=?", (replacement,))
                    database.commit()
                row = account_status(self.root, ["douyin"])[0]
                self.assertEqual(row["status"], "saved_session")
                self.assertIsNone(row["display_name"])
                self.assertTrue(row["session_present"])
                self.assertNotIn("credential_binding", row)

    def test_a_legacy_unbound_verified_snapshot_is_not_current_identity_proof(self):
        directory = self.root / "accounts"
        directory.mkdir()
        (directory / "douyin.json").write_text(json.dumps({"status": "verified", "display_name": "Old identity"}))
        row = account_status(self.root, ["douyin"])[0]
        self.assertEqual(row["status"], "saved_session")
        self.assertIsNone(row["display_name"])

    def test_new_profile_credentials_require_fresh_identity_evidence(self):
        self.verified()
        self.fixture.profile("browser-profile/macos-fixture/douyin")
        row = account_status(self.root, ["douyin"])[0]
        self.assertEqual(row["status"], "saved_session")
        self.assertIsNone(row["display_name"])


if __name__ == "__main__":
    unittest.main()
