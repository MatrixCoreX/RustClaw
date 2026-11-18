"""Test identity assertions with stub build artifacts, never compiling the UI."""
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class ProductIdentityUiArtifactsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="identity-artifact-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.bin = self.directory / "bin"
        self.bin.mkdir()
        for command in ("awk", "bash", "dirname", "env", "mktemp", "rm", "python3"):
            source = shutil.which(command)
            self.assertIsNotNone(source, command)
            (self.bin / command).symlink_to(source)
        self.assertIsNone(shutil.which("rg", path=str(self.bin)))
        stub = self.bin / "npm"
        stub.write_text(f"#!{sys.executable}\n" + '''
from pathlib import Path
import json
import os
import sys
import tomllib

assert sys.argv[1:5] == ["run", "build", "--", "--outDir"], sys.argv
mode = os.environ["IDENTITY_UI_STUB_MODE"]
if mode == "build_failed":
    sys.exit(7)
output = Path(sys.argv[5])
output.mkdir(parents=True)
fixture = Path(os.environ["APP_PRODUCT_IDENTITY_CONFIG"])
config = fixture
if mode == "wrong_second":
    config = fixture.with_name("brand-primary.toml")
name = tomllib.loads(config.read_text(encoding="utf-8"))["display_name"]
if mode != "empty":
    (output / "index.html").write_text(name if mode != "missing" else "unrelated", encoding="utf-8")
if mode == "binary_neighbor":
    (output / "binary.dat").write_bytes(bytes([255, 0, 254]))
print("IDENTITY_UI_STUB " + json.dumps({"fixture": fixture.name}))
''', encoding="utf-8")
        stub.chmod(0o700)

    def invoke(self, mode):
        environment = dict(os.environ, PATH=str(self.bin), TMPDIR=str(self.directory),
                           IDENTITY_UI_STUB_MODE=mode)
        return subprocess.run([str(self.bin / "bash"), str(ROOT / "scripts/product_identity_tests.sh"),
                               "--with-ui"], cwd=ROOT, env=environment,
                              capture_output=True, text=True, timeout=30)

    def test_both_configured_identities_are_checked_without_ripgrep(self):
        result = self.invoke("correct")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count("IDENTITY_UI_STUB "), 2)
        self.assertIn("product_identity_tests: PASS", result.stdout)

    def test_second_build_cannot_reuse_the_first_identity(self):
        result = self.invoke("wrong_second")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout.count("IDENTITY_UI_STUB "), 2)
        self.assertIn("configured identity missing", result.stderr)
        self.assertNotIn("product_identity_tests: PASS", result.stdout)

    def test_missing_identity_and_empty_output_are_rejected(self):
        for mode in ("missing", "empty"):
            with self.subTest(mode=mode):
                result = self.invoke(mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout.count("IDENTITY_UI_STUB "), 1)
                self.assertIn("configured identity missing", result.stderr)
                self.assertNotIn("product_identity_tests: PASS", result.stdout)

    def test_binary_neighbor_does_not_break_artifact_inspection(self):
        result = self.invoke("binary_neighbor")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count("IDENTITY_UI_STUB "), 2)

    def test_build_failure_is_preserved(self):
        result = self.invoke("build_failed")
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        self.assertNotIn("product_identity_tests: PASS", result.stdout)


if __name__ == "__main__":
    unittest.main()
