import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("ci_browser", ROOT / "scripts/prepare_macos_ci_browser.py")
browser = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(browser)


class PrepareMacOSBrowserTests(unittest.TestCase):
    def test_only_forbidden_metadata_is_removed_without_following_links(self):
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
            removed, visited = [], []

            def attributes(path, **kwargs):
                visited.append(Path(path))
                self.assertEqual(kwargs, {"follow_symlinks": False})
                return ["com.apple.FinderInfo", "com.apple.ResourceFork", "com.apple.quarantine", "custom"]

            with patch.object(browser.os, "listxattr", side_effect=attributes), patch.object(
                browser.os, "removexattr", side_effect=lambda path, name, **kwargs: removed.append((Path(path), name, kwargs))
            ):
                browser.remove_detritus(app)
            self.assertEqual(set(visited), {app, file})
            self.assertEqual({name for _, name, _ in removed}, browser.DETRITUS)
            self.assertEqual(len(removed), 4)
            self.assertTrue(all(kwargs == {"follow_symlinks": False} for _, _, kwargs in removed))
            self.assertEqual(file.read_bytes(), b"unchanged-signed-content")
            link = root / "bundle-link"
            link.symlink_to(app, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "symlink_rejected"):
                browser.remove_detritus(link)

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
