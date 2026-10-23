import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import zipfile

ENTRYPOINT = Path(__file__).parents[1] / "src" / "main.py"
spec = importlib.util.spec_from_file_location("image_threshold_skill", ENTRYPOINT)
skill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(skill)
OMITTED = object()


class ImageArchiveThresholdTests(unittest.TestCase):
    def request(self, root, threshold=OMITTED, mode="media"):
        args = {"action": "download", "share": "https://www.douyin.com/note/example"}
        if threshold is not OMITTED:
            args["image_archive_threshold"] = threshold
        if mode is not None:
            args["output_mode"] = mode
        return {"request_id": "archive-threshold", "args": args, "user_id": 1, "chat_id": 1,
                "context": {"workspace_root": str(root), "artifact_output_directory": str(root / "artifacts")}}

    def run_download(self, root, count, threshold, mode, video=False):
        def fake_tool(action, command, directory, *_):
            paths = []
            for index in range(count):
                path = directory / f"{index:03}.jpg"
                path.write_bytes(f"original image {index}".encode())
                paths.append(path)
            for name, body in [("post_article.txt", "平台：source\n\n正文：\nOriginal body"),
                               ("post_background_audio.mp3", "original audio")]:
                path = directory / name
                path.write_text(body)
                paths.append(path)
            if video:
                path = directory / "post.mp4"
                path.write_bytes(b"original video")
                paths.append(path)
            return "", "", [skill._artifact(path) for path in paths]
        with mock.patch.object(skill, "_run_tool", side_effect=fake_tool):
            return skill.respond(self.request(root, threshold, mode))

    def test_thresholds_zero_one_nine_and_omission_apply_in_every_output_mode(self):
        cases = [(0, 1, True), (1, 1, False), (1, 2, True), (9, 9, False),
                 (9, 10, True), (OMITTED, 9, False), (OMITTED, 10, True), (10**50, 2, False)]
        for mode in (None, "media", "media_audio"):
            for threshold, count, archived in cases:
                with self.subTest(mode=mode, threshold=threshold, count=count), tempfile.TemporaryDirectory() as folder:
                    response = self.run_download(Path(folder), count, threshold, mode)
                    self.assertEqual(response["status"], "ok")
                    artifacts = response["extra"]["artifacts"]
                    archives = [a for a in artifacts if a.get("artifact_role") == "image_archive"]
                    images = [a for a in artifacts if a.get("mime_type") == "image/jpeg"]
                    self.assertEqual(len(archives), int(archived))
                    self.assertEqual(len(images), 0 if archived else count)
                    self.assertEqual(sum(a.get("artifact_role") == "article_text" for a in artifacts), 1)
                    self.assertEqual(sum(a.get("mime_type", "").startswith("audio/") for a in artifacts), int(mode != "media"))
                    self.assertEqual(response["extra"]["content_bundle"]["image_count"], count)
                    if archived:
                        self.assertEqual(response["extra"]["content_bundle"]["image_delivery"]["threshold"],
                                         9 if threshold is OMITTED else threshold)
                        with zipfile.ZipFile(archives[0]["path"]) as archive:
                            self.assertEqual(archive.namelist(), [f"{i:03}.jpg" for i in range(count)] + ["post_article.txt"])
                            self.assertEqual(archive.read("000.jpg"), b"original image 0")

    def test_invalid_thresholds_fail_before_downloader_dispatch(self):
        for threshold in (True, False, -1, 0.5, 1.0, "1", "9", None, [], {}):
            with self.subTest(threshold=threshold), tempfile.TemporaryDirectory() as folder:
                with mock.patch.object(skill, "_run_tool") as downloader:
                    with self.assertRaises(skill.SkillFailure) as error:
                        skill.respond(self.request(Path(folder), threshold))
                    self.assertEqual(error.exception.error_code, "invalid_args")
                    self.assertEqual(error.exception.message_key, "media_download.error.invalid_image_archive_threshold")
                    downloader.assert_not_called()

    def test_zero_threshold_does_not_archive_video_audio_or_article_alone(self):
        for video in (False, True):
            with self.subTest(video=video), tempfile.TemporaryDirectory() as folder:
                response = self.run_download(Path(folder), 0, 0, "media_audio", video=video)
                artifacts = response["extra"]["artifacts"]
                self.assertFalse(any(a.get("mime_type") == "application/zip" for a in artifacts))
                self.assertTrue(any(a.get("artifact_role") == "article_text" for a in artifacts))
                self.assertTrue(any(a.get("mime_type") == "audio/mpeg" for a in artifacts))
                self.assertEqual(any(a.get("mime_type") == "video/mp4" for a in artifacts), video)


if __name__ == "__main__":
    unittest.main()
