"""Keep operator evidence private during packaging and classify staging failures."""
import errno
import importlib.util
import io
from pathlib import Path
import stat
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("release_script_staging", ROOT / "scripts/stage_release_scripts.py")
staging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(staging)


class ReleaseScriptStagingTests(unittest.TestCase):
    def test_private_operator_logs_are_not_distributable_scripts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            (source / "nl_suite_logs/private").mkdir(parents=True)
            (source / "nl_suite_logs/private/final.json").write_text('{"private":"fixture"}')
            (source / "helper.sh").write_text("#!/bin/sh\nexit 0\n")
            (source / "helper.sh").chmod(0o755)
            destination = root / "stage"
            staging.stage_scripts(source, destination)
            self.assertFalse((destination / "nl_suite_logs").exists())
            self.assertEqual((destination / "helper.sh").read_bytes(), (source / "helper.sh").read_bytes())
            self.assertTrue((destination / "helper.sh").stat().st_mode & stat.S_IXUSR)
            self.assertTrue((source / "nl_suite_logs/private/final.json").is_file())

    def test_log_root_symlink_is_excluded_without_copying_its_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            private = root / "private"
            private.mkdir()
            (private / "evidence.json").write_text("private-fixture")
            (source / "nl_suite_logs").symlink_to(private, target_is_directory=True)
            staging.stage_scripts(source, root / "stage")
            self.assertFalse((root / "stage/nl_suite_logs").exists())
            self.assertEqual((private / "evidence.json").read_text(), "private-fixture")

    def test_staging_rejects_root_links_existing_destinations_and_recursive_targets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            link = root / "source-link"
            link.symlink_to(source, target_is_directory=True)
            for src, dst in ((link, root / "stage"), (source, source / "recursive"), (source, source)):
                with self.subTest(source=src, destination=dst), self.assertRaises(ValueError):
                    staging.stage_scripts(src, dst)

    def test_safe_extraction_reports_quota_and_capacity_without_installing_files(self):
        source = (ROOT / "deploy-github-release.sh").read_text()
        marker = 'python3 - "$ARCHIVE_PATH" "$EXTRACT_DIR" "$APP_RELEASE_ARTIFACT_ID" <<\'PY\'\n'
        start = source.index(marker) + len(marker)
        snippet = compile(source[start:source.index("\nPY\n", start)], str(ROOT / "deploy-github-release.sh"), "exec")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "fixture.tar.gz"
            destination = root / "extract"
            destination.mkdir()
            with tarfile.open(archive, "w:gz") as package:
                member = tarfile.TarInfo("bundle/fixture.txt")
                member.size = 1
                package.addfile(member, io.BytesIO(b"x"))
            for error, code in ((errno.EDQUOT, "release_staging_quota_exceeded"),
                                (errno.ENOSPC, "release_staging_space_exhausted")):
                with self.subTest(error=error), mock.patch.object(sys, "argv", ["extract", str(archive), str(destination), "bundle"]):
                    with mock.patch.object(tarfile.TarFile, "extractall", side_effect=OSError(error, "fixture")):
                        with self.assertRaisesRegex(SystemExit, code):
                            exec(snippet, {})
                self.assertEqual(list(destination.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
