import importlib.util
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


TOOL_DIR = Path(__file__).parents[1] / "src/tool"
sys.path.insert(0, str(TOOL_DIR))
from macos_browser_selection import MacOSBrowser, select_macos_browser
sys.path.remove(str(TOOL_DIR))


class MacOSBrowserSelectionTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def app(self, kind="chrome", version="156.0.1", root=None):
        if kind == "chrome":
            name, binary, identifier = "Google Chrome.app", "Google Chrome", "com.google.Chrome"
        else:
            name, binary, identifier = "Microsoft Edge.app", "Microsoft Edge", "com.microsoft.edgemac"
        app = (root or self.root) / name
        executable = app / "Contents/MacOS" / binary
        executable.parent.mkdir(parents=True)
        executable.write_text("fixture")
        executable.chmod(0o700)
        with (app / "Contents/Info.plist").open("wb") as f:
            plistlib.dump({"CFBundleIdentifier": identifier, "CFBundleExecutable": binary,
                          "CFBundleShortVersionString": version}, f)
        return executable

    def choose(self, **kwargs):
        return select_macos_browser(platform_name="darwin", applications=(self.root,), **kwargs)

    @mock.patch("macos_browser_selection.subprocess.run")
    def test_current_signed_chrome_is_selected_without_launching_it(self, run):
        executable = self.app()
        selected = self.choose()
        self.assertEqual(selected.executable, str(executable))
        self.assertEqual(selected.profile_namespace, "macos-com.google.Chrome")
        command = run.call_args.args[0]
        self.assertEqual(command[0], "/usr/bin/codesign")
        self.assertIn('certificate leaf[subject.OU] = "EQHXZ8M8AV"', command[4])
        self.assertEqual(run.call_args.kwargs["timeout"], 5)

    @mock.patch("macos_browser_selection.subprocess.run")
    def test_obsolete_chrome_does_not_hide_supported_edge(self, run):
        self.app(version="65.0.3325.181")
        edge = self.app("edge", "155.0.1")
        self.assertEqual(self.choose().executable, str(edge))
        self.assertEqual(run.call_count, 1)

    @mock.patch("macos_browser_selection.subprocess.run")
    def test_invalid_signature_falls_back_to_signed_edge(self, run):
        self.app()
        edge = self.app("edge")
        run.side_effect = [subprocess.CalledProcessError(1, "codesign"), mock.Mock()]
        self.assertEqual(self.choose().executable, str(edge))

    @mock.patch("macos_browser_selection.subprocess.run")
    def test_signature_timeout_is_bounded_and_not_a_launch(self, run):
        self.app()
        run.side_effect = subprocess.TimeoutExpired("codesign", 5)
        self.assertIsNone(self.choose())

    @mock.patch("macos_browser_selection.subprocess.run")
    def test_user_applications_and_architecture_neutral_selection(self, run):
        user = self.root / "user/Applications"
        executable = self.app(root=user)
        selected = select_macos_browser(platform_name="darwin", applications=(user, self.root))
        self.assertEqual(selected.executable, str(executable))

    @mock.patch("macos_browser_selection.subprocess.run")
    def test_missing_or_forged_metadata_is_not_accepted(self, run):
        executable = self.app()
        info = executable.parents[1] / "Info.plist"
        info.write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.google.Chrome",
                                       "CFBundleExecutable": "../different",
                                       "CFBundleShortVersionString": "156.0"}))
        self.assertIsNone(self.choose())
        run.assert_not_called()

    @mock.patch("macos_browser_selection.subprocess.run")
    def test_linux_and_missing_browsers_never_execute_macos_commands(self, run):
        self.app()
        self.assertIsNone(select_macos_browser(platform_name="linux", applications=(self.root,)))
        self.assertIsNone(select_macos_browser(platform_name="darwin", applications=(self.root / "absent",)))
        run.assert_not_called()

    def test_browser_encryption_identities_have_different_profile_names(self):
        chrome = MacOSBrowser("/fixture/chrome", "com.google.Chrome")
        edge = MacOSBrowser("/fixture/edge", "com.microsoft.edgemac")
        self.assertNotEqual(chrome.profile_namespace, edge.profile_namespace)
        self.assertNotIn("/", chrome.profile_namespace)


class DownloadBrowserCommandTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        tool = TOOL_DIR.parent / "main.py"
        spec = importlib.util.spec_from_file_location("browser_selection_skill", tool)
        cls.skill = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.skill)

    def test_native_browser_path_and_private_profile_are_bound_together(self):
        browser = MacOSBrowser("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "com.google.Chrome")
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(
            self.skill, "select_macos_browser", return_value=browser,
        ):
            storage = Path(directory) / "storage"
            command = self.skill._build_download_command(
                {"share": "https://example.test/media"}, Path(directory) / "output",
                resolve_only=False, storage_directory=storage,
            )
            self.assertEqual(command[command.index("--chrome-path") + 1], browser.executable)
            self.assertEqual(command[command.index("--browser-profile-dir") + 1],
                             str(storage / "browser-profile/macos-com.google.Chrome"))
            self.assertIn("--no-system-browser-cookies", command)

    def test_cli_explicit_override_still_wins(self):
        spec = importlib.util.spec_from_file_location("browser_selection_downloader", TOOL_DIR / "media_downloader.py")
        tool = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = tool
        sys.path.insert(0, str(TOOL_DIR))
        try:
            spec.loader.exec_module(tool)
        finally:
            sys.path.remove(str(TOOL_DIR))
        with tempfile.NamedTemporaryFile() as executable, mock.patch.object(tool.sys, "platform", "darwin"):
            self.assertEqual(tool.find_chrome_executable(executable.name), executable.name)


if __name__ == "__main__":
    unittest.main()
