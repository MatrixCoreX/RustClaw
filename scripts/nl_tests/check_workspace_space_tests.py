import types
import unittest
from unittest.mock import Mock, patch

from check_workspace_space import check_space


class CopySpaceTests(unittest.TestCase):
    def path(self, device):
        path = Mock()
        path.stat.return_value.st_dev = device
        return path

    @patch("check_workspace_space.shutil.disk_usage")
    def test_same_device_must_fit_source_and_snapshot_together(self, usage):
        usage.return_value = types.SimpleNamespace(free=100)
        with self.assertRaisesRegex(ValueError, "copy_space_unavailable"):
            check_space([(self.path(1), 60), (self.path(1), 60)])

    @patch("check_workspace_space.shutil.disk_usage")
    def test_separate_devices_have_independent_budgets(self, usage):
        usage.return_value = types.SimpleNamespace(free=60)
        check_space([(self.path(1), 60), (self.path(2), 60)])
        self.assertEqual(usage.call_count, 2)

    @patch("check_workspace_space.shutil.disk_usage")
    def test_large_snapshot_volume_does_not_hide_small_state_volume(self, usage):
        state, snapshot = self.path(1), self.path(2)
        usage.side_effect = lambda path: types.SimpleNamespace(free=20 if path is state else 1000)
        with self.assertRaisesRegex(ValueError, "copy_space_unavailable"):
            check_space([(state, 30), (snapshot, 60)])


if __name__ == "__main__":
    unittest.main()
