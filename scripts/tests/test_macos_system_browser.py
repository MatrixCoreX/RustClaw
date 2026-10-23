from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]


class MacOSSystemBrowserTest(unittest.TestCase):
    def test_launcher_passes_literal_requirement_quotes_and_a_real_path_newline(self):
        lines = (ROOT / "scripts/macos-system-browser.sh").read_text().splitlines()
        requirement = next(line for line in lines if line.strip().startswith("requirement="))
        checked = subprocess.run(
            ["/bin/bash", "-c", 'bundle_id=com.fixture; team=FIXTURE;\n'
             + requirement + '\nprintf "%s" "$requirement"'],
            capture_output=True, text=True, check=True,
        )
        self.assertEqual(checked.stdout,
                         '=anchor apple generic and identifier "com.fixture" '
                         'and certificate leaf[subject.OU] = "FIXTURE"')
        projection = next(line for line in lines if line.strip().startswith("printf"))
        resolved = subprocess.run(
            ["/bin/bash", "-c", 'executable=/fixture/browser;\n' + projection],
            capture_output=True, check=True,
        )
        self.assertEqual(resolved.stdout, b"/fixture/browser\n")

    def test_launcher_is_valid_shell_and_preserves_encryption_and_arguments(self):
        launcher = ROOT / "scripts/macos-system-browser.sh"
        subprocess.run(["bash", "-n", str(launcher)], check=True)
        source = launcher.read_text()
        self.assertIn('exec "$executable" "$@"', source)
        self.assertIn("/usr/bin/codesign --verify --strict", source)
        self.assertIn("--resolve-executable", source)
        for forbidden in ("--use-mock-keychain", "--password-store=basic", "--no-sandbox",
                          "security delete", "security set-key-partition-list", "Chrome for Testing"):
            self.assertNotIn(forbidden, source)

    def test_macos_artifact_no_longer_bundles_or_installs_testing_chromium(self):
        bundler = (ROOT / "scripts/bundle_macos_skill_dependencies.py").read_text()
        workflow = (ROOT / ".github/workflows/macos-arm64-release-artifact.yml").read_text()
        self.assertIn('"chromium": "system_browser_required"', bundler)
        self.assertNotIn('parser.add_argument("--chromium"', bundler)
        self.assertNotIn("playwright install chromium", workflow)
        self.assertIn('"${host_tools}/bin/chromium" --resolve-executable', workflow)

    @unittest.skipUnless(sys.platform == "darwin", "Native signature and application test requires macOS")
    def test_native_launcher_resolves_a_signed_system_app(self):
        script = ROOT / "scripts/macos-system-browser.sh"
        selected = subprocess.run(["/bin/bash", str(script), "--resolve-executable"],
                                  capture_output=True, text=True, timeout=30)
        if selected.returncode == 127:
            self.assertIn('"error_code":"system_browser_unavailable"', selected.stderr)
            return
        self.assertEqual(selected.returncode, 0, selected.stderr)
        self.assertTrue(selected.stdout.strip().endswith(("/Contents/MacOS/Google Chrome",
                                                       "/Contents/MacOS/Microsoft Edge")))
        python_selector = subprocess.run(
            [sys.executable, "-c",
             "import sys; sys.path.insert(0, sys.argv[1]); "
             "from macos_browser_selection import select_macos_browser; "
             "browser=select_macos_browser(); "
             "print(browser.executable if browser else '')",
             str(ROOT / "optional_skills/media_download/src/tool")],
            capture_output=True, text=True, timeout=30, check=True,
        )
        self.assertEqual(python_selector.stdout.strip(), selected.stdout.strip())
        version = subprocess.run(["/bin/bash", str(script), "--version"],
                                 capture_output=True, text=True, timeout=30, check=True)
        self.assertTrue(version.stdout.strip())


if __name__ == "__main__":
    unittest.main()
