"""Build-time Linux Python portability and strict version regressions.

The real-runtime cases are enabled by the native Linux release workflow after
its upstream archive SHA is verified; they never download or build on users'
nodes. The nonportable control is Actions' original setup-python distribution.
"""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/verify_linux_python_runtime.py"
PORTABLE = os.environ.get("APP_TEST_LINUX_PYTHON_RUNTIME_ROOT")
NONPORTABLE = os.environ.get("APP_TEST_NONPORTABLE_PYTHON_RUNTIME_ROOT")


class LinuxPythonReleaseTests(unittest.TestCase):
    def test_native_linux_workflow_and_package_gate_use_the_selected_runtime(self):
        workflow = (ROOT / ".github/workflows/ubuntu-x86_64-release.yml").read_text()
        prepare = workflow.index("name: Prepare locked relocatable Linux Python runtime")
        precompile = workflow.index("name: Precompile platform Skill Store packages")
        self.assertLess(prepare, precompile)
        self.assertIn('echo "APP_PYTHON_BIN=${runtime_root}/bin/python3"', workflow)
        self.assertIn('python_runtime_root="${APP_RELEASE_PYTHON_RUNTIME_ROOT:', workflow)
        self.assertNotIn('python_runtime_root="${pythonLocation:', workflow)
        self.assertIn("sha256sum -c -", workflow[prepare:precompile])
        self.assertIn('python-version: "3.13.16"', workflow)
        self.assertIn('echo "APP_TEST_LINUX_PYTHON_RUNTIME_ROOT=${runtime_root}"', workflow)
        self.assertIn('echo "APP_TEST_NONPORTABLE_PYTHON_RUNTIME_ROOT=${pythonLocation:', workflow)
        self.assertLess(prepare, workflow.index("name: Test Release installation boundaries"))
        self.assertIn("-p 'test_release*.py'", workflow)
        package = (ROOT / "package-release.sh").read_text()
        self.assertLess(package.index("scripts/verify_linux_python_runtime.py"),
                        package.index("bundled-skill-bootstrap-v1.json"))
        self.assertIn('if [[ "$APP_PACKAGE_TARGET" == *-linux-* ]]', package)

        gate = package.index("scripts/verify_linux_python_runtime.py")
        full_only = package.rfind('if [[ "$INSTALL_ALL_BUNDLED_SKILLS" == "1" ]]; then', 0, gate)
        self.assertGreater(full_only, 0)
        self.assertLess(gate, package.index("\nfi\n", full_only))
        self.assertIn('INSTALL_ALL_BUNDLED_SKILLS="${APP_RELEASE_INSTALL_ALL_BUNDLED_SKILLS:-0}"', package)
        pi = (ROOT / ".github/workflows/pi-aarch64-release.yml").read_text()
        self.assertIn('APP_PACKAGE_TARGET="${RUST_TARGET}"', pi)
        self.assertNotIn("APP_RELEASE_INSTALL_ALL_BUNDLED_SKILLS", pi)
        self.assertNotIn("APP_RELEASE_PYTHON_RUNTIME_ROOT", pi)

    @unittest.skipUnless(sys.platform == "linux" and PORTABLE, "verified Linux Python runtime not supplied")
    def test_relocated_base_copied_and_sdk_rebound_venvs_without_source_or_toolcache(self):
        completed = subprocess.run(
            [sys.executable, str(SCRIPT), PORTABLE, "--expected-version", "3.13.16"],
            capture_output=True, text=True, timeout=120,
        )
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        proof = json.loads(completed.stdout)
        self.assertTrue(proof["source_hidden"])
        self.assertTrue(proof["toolcache_hidden"])
        for name, copied in [("base", False), ("copied_venv", True), ("sdk_rebound_venv", True)]:
            with self.subTest(name=name):
                self.assertEqual(proof["results"][name],
                                 {"version": "3.13.16", "copied": copied, "stdlib": True})

    @unittest.skipUnless(sys.platform == "linux" and NONPORTABLE, "nonportable Linux Python control not supplied")
    def test_ci_toolcache_runtime_is_rejected_after_relocation(self):
        completed = subprocess.run(
            [sys.executable, str(SCRIPT), NONPORTABLE],
            capture_output=True, text=True, timeout=120,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("Linux Python relocation/copied-venv verification failed", completed.stderr)

    @unittest.skipUnless(sys.platform == "linux" and PORTABLE, "verified Linux Python runtime not supplied")
    def test_wrong_adapter_version_is_rejected_instead_of_falling_back(self):
        spec = importlib.util.spec_from_file_location("linux_python_gate", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with self.assertRaisesRegex(ValueError, "version mismatch"):
            module.verify(PORTABLE, "0.0.0")


if __name__ == "__main__":
    unittest.main()
