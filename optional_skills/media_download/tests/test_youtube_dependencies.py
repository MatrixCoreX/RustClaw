from pathlib import Path
import re
import unittest


SKILL_ROOT = Path(__file__).parents[1]
YOUTUBE_DEPENDENCIES = ("yt-dlp==2026.8.19", "yt-dlp-ejs==0.8.0")


class YoutubeDependencyTest(unittest.TestCase):
    def test_native_keyring_support_is_scoped_to_linux(self) -> None:
        requirements = (SKILL_ROOT / "requirements.in").read_text().splitlines()
        self.assertIn('secretstorage==3.5.0; sys_platform == "linux"', requirements)

    def test_native_keyring_support_is_hashed_and_platform_scoped(self) -> None:
        lock = (SKILL_ROOT / "requirements.lock").read_text()
        self.assertRegex(
            lock,
            r"(?m)^secretstorage==3\.5\.0 ; sys_platform == 'linux' \\\n    --hash=sha256:[a-f0-9]{64}",
        )

    def test_compatible_downloader_and_solver_are_explicit_requirements(self) -> None:
        requirements = (SKILL_ROOT / "requirements.in").read_text().splitlines()
        for dependency in YOUTUBE_DEPENDENCIES:
            with self.subTest(dependency=dependency):
                self.assertIn(dependency, requirements)

    def test_downloader_and_solver_are_locked_with_verified_hashes(self) -> None:
        lock = (SKILL_ROOT / "requirements.lock").read_text()
        for dependency in YOUTUBE_DEPENDENCIES:
            with self.subTest(dependency=dependency):
                self.assertRegex(
                    lock,
                    rf"(?m)^{re.escape(dependency)} \\\n    --hash=sha256:[a-f0-9]{{64}}",
                )


if __name__ == "__main__":
    unittest.main()
