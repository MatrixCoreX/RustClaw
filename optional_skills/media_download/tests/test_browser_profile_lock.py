from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
sys.path.insert(0, str(TOOL_DIR))
from browser_profile_lock import profile_lease, serialize_profile
sys.path.remove(str(TOOL_DIR))


class BrowserProfileLockTest(unittest.TestCase):
    def test_alias_and_default_platform_use_the_canonical_profile_lock(self):
        normalize = lambda value: {"alias": "canonical"}.get(value, value)

        @serialize_profile("browser_profile_dir", platform_subdir=normalize)
        def invoke(*, browser_profile_dir, platform="alias"):
            self.assertTrue((Path(browser_profile_dir) / "canonical" / ".agent-profile.lock").is_file())
            self.assertFalse((Path(browser_profile_dir) / "alias").exists())

        with tempfile.TemporaryDirectory() as root:
            invoke(browser_profile_dir=root)
            invoke(browser_profile_dir=root, platform="canonical")

    def test_profile_lock_is_reentrant_and_released_after_error(self):
        with tempfile.TemporaryDirectory() as root:
            profile = Path(root)
            with self.assertRaises(ValueError):
                with profile_lease(profile, timeout=0.1):
                    with profile_lease(profile, timeout=0.1):
                        raise ValueError("fixture")
            with profile_lease(profile, timeout=0.1):
                pass

    def test_second_process_cannot_use_same_profile_but_other_profile_is_free(self):
        code = (
            "import sys; from pathlib import Path; "
            "from browser_profile_lock import profile_lease; "
            "lease=profile_lease(Path(sys.argv[1]),timeout=0.15); "
            "lease.__enter__(); lease.__exit__(None,None,None)"
        )
        with tempfile.TemporaryDirectory() as root:
            profile = Path(root) / "one"
            with profile_lease(profile):
                blocked = subprocess.run([sys.executable, "-c", code, str(profile)], cwd=TOOL_DIR, capture_output=True, timeout=5)
                separate = subprocess.run([sys.executable, "-c", code, str(Path(root) / "two")], cwd=TOOL_DIR, capture_output=True, timeout=5)
            released = subprocess.run([sys.executable, "-c", code, str(profile)], cwd=TOOL_DIR, capture_output=True, timeout=5)
        self.assertNotEqual(blocked.returncode, 0)
        self.assertIn(b"browser_profile_busy", blocked.stderr)
        self.assertEqual(separate.returncode, 0, separate.stderr)
        self.assertEqual(released.returncode, 0, released.stderr)
