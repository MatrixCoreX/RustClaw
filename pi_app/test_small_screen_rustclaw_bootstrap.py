import os
import stat
import tempfile
import unittest
from unittest import mock

import small_screen_rustclaw_bootstrap as bootstrap


def _touch_executable(path):
    with open(path, "w", encoding="utf-8") as f:
        f.write("#!/bin/sh\nexit 0\n")
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


class RustClawBootstrapTests(unittest.TestCase):
    def test_running_instance_does_not_spawn(self):
        calls = []
        with tempfile.TemporaryDirectory() as root:
            with mock.patch.object(bootstrap, "rustclaw_is_running", return_value=True):
                started = bootstrap.ensure_rustclaw_started(
                    root=root,
                    wait_seconds=0,
                    popen_factory=lambda *args, **kwargs: calls.append((args, kwargs)),
                )
        self.assertFalse(started)
        self.assertEqual(calls, [])

    def test_missing_instance_spawns_cli_restart(self):
        calls = []
        with tempfile.TemporaryDirectory() as root:
            cli = os.path.join(root, "rustclaw")
            _touch_executable(cli)

            def fake_popen(cmd, **kwargs):
                calls.append((cmd, kwargs))

            with mock.patch.object(bootstrap, "rustclaw_is_running", return_value=False):
                started = bootstrap.ensure_rustclaw_started(
                    root=root,
                    wait_seconds=0,
                    popen_factory=fake_popen,
                )

        self.assertTrue(started)
        self.assertEqual(
            calls[0][0],
            [cli, "-restart", "release", "all", "--quick", "--skip-setup"],
        )
        self.assertEqual(calls[0][1]["cwd"], root)
        self.assertTrue(calls[0][1]["start_new_session"])

    def test_missing_cli_falls_back_to_start_all_bin(self):
        calls = []
        with tempfile.TemporaryDirectory() as root:
            start_all = os.path.join(root, "start-all-bin.sh")
            with open(start_all, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\n")

            def fake_popen(cmd, **kwargs):
                calls.append((cmd, kwargs))

            with mock.patch.object(bootstrap, "rustclaw_is_running", return_value=False):
                started = bootstrap.ensure_rustclaw_started(
                    root=root,
                    wait_seconds=0,
                    popen_factory=fake_popen,
                )

        self.assertTrue(started)
        self.assertEqual(calls[0][0], ["bash", start_all, "release"])

    def test_custom_start_command_takes_precedence(self):
        with mock.patch.dict(
            os.environ,
            {"RUSTCLAW_SMALL_SCREEN_START_CMD": "bash /tmp/start-rustclaw.sh --fast"},
        ):
            self.assertEqual(
                bootstrap._default_start_command("/tmp/missing-root"),
                ["bash", "/tmp/start-rustclaw.sh", "--fast"],
            )


if __name__ == "__main__":
    unittest.main()
