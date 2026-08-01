import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "media_downloader.py"


def load_downloader_module():
    spec = importlib.util.spec_from_file_location(
        "media_download_profile_checkpoint_tool",
        ENTRYPOINT,
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class ProfileCheckpointTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.downloader = load_downloader_module()

    def _args(self, output_dir: Path, checkpoint_dir: Path):
        return SimpleNamespace(
            browser_fallback=True,
            profile_limit=2,
            profile_interval=0.0,
            browser_timeout=1.0,
            chrome_path=None,
            system_browser_cookies=False,
            verbose=False,
            print_url=False,
            output_dir=str(output_dir),
            output_name=None,
            profile_checkpoint_dir=str(checkpoint_dir),
        )

    def test_partial_profile_resume_restores_completed_item_without_redownload(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output_dir = root / "first-output"
            checkpoint_dir = root / "private-checkpoints"
            result = self.downloader.DouyinProfileResult(
                sec_uid="stable-profile-id",
                username="Creator",
                posts=[
                    self.downloader.DouyinProfilePost("1001", 0, {}),
                    self.downloader.DouyinProfilePost("1002", 0, {}),
                ],
                logs=[],
            )
            calls: list[str] = []
            incomplete = True

            def fake_download(
                item_args,
                _item_url,
                _cookie,
                _platform,
                item_id,
                _candidates,
                _image_candidates,
                _logs,
                *,
                article=None,
            ):
                del article
                calls.append(item_id)
                if incomplete and item_id == "1002":
                    return 0
                target = Path(item_args.output_dir)
                target.mkdir(parents=True, exist_ok=True)
                (target / item_args.output_name).write_bytes(f"media-{item_id}".encode())
                return 0

            candidates = [self.downloader.Candidate("https://media.test/video", "test", 1)]
            with (
                mock.patch.object(
                    self.downloader,
                    "gather_douyin_profile_posts",
                    return_value=result,
                ),
                mock.patch.object(
                    self.downloader,
                    "profile_post_media_candidates",
                    return_value=(candidates, []),
                ),
                mock.patch.object(
                    self.downloader,
                    "handle_resolved_media",
                    side_effect=fake_download,
                ),
            ):
                with self.assertRaisesRegex(
                    self.downloader.DouyinDownloadError,
                    "Profile collection is partial",
                ):
                    self.downloader.handle_douyin_profile(
                        self._args(output_dir, checkpoint_dir),
                        "https://www.douyin.com/user/stable-profile-id",
                        "stable-profile-id",
                        None,
                    )

                checkpoint_root = self.downloader.profile_checkpoint_root(
                    self._args(output_dir, checkpoint_dir),
                    "douyin",
                    "stable-profile-id",
                )
                assert checkpoint_root is not None
                partial_snapshots = sorted(
                    (checkpoint_root / self.downloader.PROFILE_CHECKPOINT_SNAPSHOT_FOLDER).glob("*.json")
                )
                self.assertTrue(partial_snapshots)
                oldest_snapshot = partial_snapshots[0]
                oldest_content = oldest_snapshot.read_bytes()

                shutil.rmtree(output_dir)
                incomplete = False
                self.assertEqual(
                    self.downloader.handle_douyin_profile(
                        self._args(output_dir, checkpoint_dir),
                        "https://www.douyin.com/user/stable-profile-id",
                        "stable-profile-id",
                        None,
                    ),
                    0,
                )
                snapshot_count = len(
                    list(
                        (
                            checkpoint_root
                            / self.downloader.PROFILE_CHECKPOINT_SNAPSHOT_FOLDER
                        ).glob("*.json")
                    )
                )
                self.assertEqual(
                    self.downloader.handle_douyin_profile(
                        self._args(output_dir, checkpoint_dir),
                        "https://www.douyin.com/user/stable-profile-id",
                        "stable-profile-id",
                        None,
                    ),
                    0,
                )
                self.assertEqual(
                    len(
                        list(
                            (
                                checkpoint_root
                                / self.downloader.PROFILE_CHECKPOINT_SNAPSHOT_FOLDER
                            ).glob("*.json")
                        )
                    ),
                    snapshot_count,
                )

            self.assertEqual(calls.count("1001"), 1)
            self.assertEqual(calls.count("1002"), 2)
            self.assertEqual(oldest_snapshot.read_bytes(), oldest_content)
            manifest_path = output_dir / "Creator" / "profile_downloads.json"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            self.assertEqual(manifest["state"], "complete")
            self.assertEqual(manifest["item_count"], 2)
            self.assertEqual(manifest["completed_count"], 2)
            self.assertEqual(manifest["failed_count"], 0)
            self.assertEqual(manifest["collection"]["cursor"]["next_index"], 2)
            self.assertEqual(
                manifest["collection"]["stable_item_ids"],
                ["1001", "1002"],
            )
            restored = [
                path
                for path in (output_dir / "Creator").rglob("*.mp4")
                if path.is_file()
            ]
            self.assertEqual(len(restored), 2)
            pointer = json.loads(
                (checkpoint_root / self.downloader.PROFILE_CHECKPOINT_POINTER_FILENAME).read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(pointer["state"], "complete")
            self.assertEqual(pointer["sha256"], manifest["checkpoint_digest"])
            cursor_indexes = [
                json.loads(path.read_text(encoding="utf-8"))["collection"]["cursor"][
                    "next_index"
                ]
                for path in sorted(
                    (
                        checkpoint_root
                        / self.downloader.PROFILE_CHECKPOINT_SNAPSHOT_FOLDER
                    ).glob("*.json")
                )
            ]
            self.assertEqual(cursor_indexes, sorted(cursor_indexes))

    def test_corrupt_cached_artifact_is_rejected_instead_of_silently_replayed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output_dir = root / "output"
            output_dir.mkdir()
            source = output_dir / "videos" / "item.mp4"
            source.parent.mkdir()
            source.write_bytes(b"original")
            checkpoint_root = root / "checkpoint"
            descriptors = self.downloader.cache_profile_item_files(
                checkpoint_root,
                output_dir,
                ["videos/item.mp4"],
            )
            source.unlink()
            blob = (
                checkpoint_root
                / self.downloader.PROFILE_CHECKPOINT_BLOB_FOLDER
                / descriptors[0]["sha256"]
            )
            blob.write_bytes(b"corrupt")

            with self.assertRaisesRegex(
                self.downloader.DouyinDownloadError,
                "checkpoint artifact is unavailable",
            ):
                self.downloader.restore_profile_item_files(
                    checkpoint_root,
                    output_dir,
                    {"artifacts": descriptors},
                )

    def test_xiaohongshu_profile_uses_the_same_complete_checkpoint_contract(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output_dir = root / "output"
            checkpoint_dir = root / "private-checkpoints"
            result = self.downloader.XiaohongshuProfileResult(
                user_id="stable-xhs-profile",
                username="Writer",
                posts=[
                    self.downloader.XiaohongshuProfilePost(
                        "66a1b2c3d4e5f60718293abc",
                        0,
                        "token",
                        "normal",
                        {},
                    )
                ],
                logs=[],
            )

            def fake_download(
                item_args,
                _item_url,
                _cookie,
                _platform,
                item_id,
                _candidates,
                _image_candidates,
                _logs,
                *,
                article=None,
            ):
                del article
                target = Path(item_args.output_dir)
                target.mkdir(parents=True, exist_ok=True)
                (target / item_args.output_name).write_bytes(item_id.encode())
                return 0

            with (
                mock.patch.object(
                    self.downloader,
                    "gather_xiaohongshu_profile_posts",
                    return_value=result,
                ),
                mock.patch.object(
                    self.downloader,
                    "gather_candidates_for_request_with_retries",
                    return_value=(
                        "xiaohongshu",
                        result.posts[0].item_id,
                        [self.downloader.Candidate("https://media.test/video", "test", 1)],
                        [],
                        None,
                        [],
                    ),
                ),
                mock.patch.object(
                    self.downloader,
                    "handle_resolved_media",
                    side_effect=fake_download,
                ),
            ):
                self.assertEqual(
                    self.downloader.handle_xiaohongshu_profile(
                        self._args(output_dir, checkpoint_dir),
                        "https://www.xiaohongshu.com/user/profile/stable-xhs-profile",
                        "stable-xhs-profile",
                        None,
                    ),
                    0,
                )

            manifest = json.loads(
                (output_dir / "Writer" / "profile_downloads.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(manifest["state"], "complete")
            self.assertEqual(manifest["platform"], "xiaohongshu")
            self.assertEqual(manifest["completed_count"], 1)
            self.assertEqual(manifest["failed_count"], 0)


if __name__ == "__main__":
    unittest.main()
