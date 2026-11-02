import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("ci_browser", ROOT / "scripts/prepare_macos_ci_browser.py")
browser = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(browser)


class PrepareMacOSBrowserTests(unittest.TestCase):
    def test_native_commands_remove_only_explicit_metadata_and_do_not_follow_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app = root / "browser.app"
            app.mkdir()
            file = app / "signed-binary"
            file.write_bytes(b"unchanged-signed-content")
            outside = root / "outside"
            outside.mkdir()
            (app / "outside-link").symlink_to(outside, target_is_directory=True)
            (app / "file-link").symlink_to(file)
            with patch.object(browser.subprocess, "run", return_value=Mock(returncode=0)) as run:
                browser.remove_detritus(app)
            self.assertEqual([call.args[0] for call in run.call_args_list], [
                ["/usr/bin/xattr", "-r", "-s", "-d", attribute, str(app)]
                for attribute in browser.DETRITUS
            ])
            self.assertEqual(file.read_bytes(), b"unchanged-signed-content")
            link = root / "bundle-link"
            link.symlink_to(app, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "symlink_rejected"):
                browser.remove_detritus(link)

    @unittest.skipUnless(sys.platform == "darwin", "Checks actual macOS filesystem attributes")
    def test_native_metadata_repair_preserves_quarantine_content_and_external_targets(self):
        def xattr(*args, check=True):
            return subprocess.run(["/usr/bin/xattr", *map(str, args)], capture_output=True, check=check)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app, outside = root / "browser.app", root / "outside"
            app.mkdir()
            outside.mkdir()
            inside_file, outside_file = app / "signed-binary", outside / "private-file"
            inside_file.write_bytes(b"unchanged-signed-content")
            outside_file.write_bytes(b"external-content")
            (app / "directory-link").symlink_to(outside, target_is_directory=True)
            (app / "file-link").symlink_to(outside_file)
            for file in (inside_file, outside_file):
                xattr("-wx", "com.apple.FinderInfo", "01" * 32, file)
            xattr("-w", "com.apple.ResourceFork", "fixture-fork", inside_file)
            xattr("-w", "com.apple.quarantine", "fixture-quarantine", inside_file)
            quarantine = xattr("-p", "com.apple.quarantine", inside_file).stdout
            browser.remove_detritus(app)
            for attribute in browser.DETRITUS:
                self.assertNotEqual(xattr("-p", attribute, inside_file, check=False).returncode, 0)
            self.assertEqual(xattr("-p", "com.apple.quarantine", inside_file).stdout, quarantine)
            self.assertEqual(xattr("-p", "com.apple.FinderInfo", outside_file).returncode, 0)
            self.assertEqual(inside_file.read_bytes(), b"unchanged-signed-content")
            self.assertEqual(outside_file.read_bytes(), b"external-content")

    def test_local_and_non_macos_invocations_do_not_run_or_mutate_anything(self):
        for platform, actions in [("linux", "true"), ("darwin", "false"), ("darwin", "")]:
            with self.subTest(platform=platform, actions=actions), patch.object(browser.sys, "platform", platform), patch.dict(
                browser.os.environ, {"GITHUB_ACTIONS": actions}
            ), patch.object(browser, "browser_available") as probe, patch.object(browser, "remove_detritus") as repair, patch.object(browser.subprocess, "run") as command:
                with self.assertRaisesRegex(RuntimeError, "requires_macos_github_runner"):
                    browser.main()
                probe.assert_not_called()
                repair.assert_not_called()
                command.assert_not_called()

    def prepare(self, availability, installed=True):
        commands = []

        def run(args, **kwargs):
            commands.append((args, kwargs))
            return Mock(returncode=0 if installed else 1)

        with patch.object(browser.sys, "platform", "darwin"), patch.dict(browser.os.environ, {"GITHUB_ACTIONS": "true"}), patch.object(
            browser, "browser_available", side_effect=availability
        ), patch.object(browser, "remove_detritus") as repair, patch.object(browser.subprocess, "run", side_effect=run):
            browser.main()
        return commands, repair

    def test_valid_browser_is_not_modified(self):
        commands, repair = self.prepare([True])
        self.assertEqual(commands, [])
        repair.assert_not_called()

    def test_metadata_only_repair_avoids_reinstall(self):
        commands, repair = self.prepare([False, True])
        self.assertEqual(commands, [])
        repair.assert_called_once_with(browser.CHROME)

    def test_reinstall_and_install_both_require_a_final_strict_probe(self):
        for installed in [True, False]:
            commands, repair = self.prepare([False, False, True], installed)
            self.assertEqual(commands[1][0], ["brew", "reinstall" if installed else "install", "--cask", "google-chrome"])
            self.assertTrue(commands[1][1]["check"])
            self.assertEqual(commands[1][1]["env"]["HOMEBREW_NO_AUTO_UPDATE"], "1")
            self.assertEqual(repair.call_count, 2)
        with self.assertRaisesRegex(RuntimeError, "signed_system_browser_selection_failed"):
            self.prepare([False, False, False])

    def test_probe_delegates_to_the_runtime_signature_verifier(self):
        with patch.object(browser.subprocess, "run", return_value=Mock(returncode=127)) as run:
            self.assertFalse(browser.browser_available())
        self.assertEqual(run.call_args.args[0], ["/bin/bash", str(ROOT / "scripts/macos-system-browser.sh"), "--resolve-executable"])

    def test_install_failure_cannot_be_reported_as_ready(self):
        failure = subprocess.CalledProcessError(1, ["brew", "reinstall"])
        with patch.object(browser.sys, "platform", "darwin"), patch.dict(browser.os.environ, {"GITHUB_ACTIONS": "true"}), patch.object(
            browser, "browser_available", side_effect=[False, False]
        ), patch.object(browser, "remove_detritus"), patch.object(
            browser.subprocess, "run", side_effect=[Mock(returncode=0), failure]
        ):
            with self.assertRaises(subprocess.CalledProcessError):
                browser.main()

    def test_cleanup_error_is_not_converted_into_verification_success(self):
        with patch.object(browser, "remove_detritus", side_effect=PermissionError), patch.object(browser.sys, "stderr") as stderr:
            browser.clean_browser_metadata()
        self.assertTrue(stderr.write.called)


if __name__ == "__main__":
    unittest.main()
