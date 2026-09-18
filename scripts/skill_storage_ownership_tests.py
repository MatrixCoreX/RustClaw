#!/usr/bin/env python3
"""Storage scans must distinguish retained NL workspaces from production code."""
import tempfile
import unittest
from pathlib import Path

from check_skill_storage_ownership import evaluate, production_rust_files, write_fixture


class StorageScanTests(unittest.TestCase):
    def test_retained_workspace_is_excluded_but_nested_production_tmp_is_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            retained = root / "tmp/nl-isolated/workspace/crates/clawd/src/repo/crypto_storage.rs"
            retained.parent.mkdir(parents=True)
            retained.write_text("exchange_api_credentials\n")
            self.assertEqual(evaluate(root), [])
            self.assertNotIn(retained, production_rust_files(root))

            production = root / "crates/clawd/src/tmp/storage.rs"
            production.parent.mkdir(parents=True)
            production.write_text("exchange_api_credentials\n")
            self.assertIn(production, production_rust_files(root))
            self.assertIn(
                "crypto_table_outside_owner:crates/clawd/src/tmp/storage.rs", evaluate(root)
            )

    def test_existing_build_and_test_exclusions_do_not_hide_untracked_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in (
                "target/debug/generated.rs", "external_skills/demo/target/generated.rs",
                "crates/demo/tests/fixture.rs", "crates/demo/src/main_tests.rs",
                "crates/demo/src/untracked.rs",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("database_sqlite_path\n")
            self.assertEqual(production_rust_files(root), [root / "crates/demo/src/untracked.rs"])


if __name__ == "__main__":
    unittest.main()
