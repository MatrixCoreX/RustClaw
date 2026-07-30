import importlib.util
from pathlib import Path
import sys
import unittest


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "media_downloader.py"


def load_downloader_module():
    spec = importlib.util.spec_from_file_location("media_download_share_text_tool", ENTRYPOINT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOL_DIR))
    return module


class ShareTextTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.downloader = load_downloader_module()

    def assert_share_text(self, text: str, expected_url: str, expected_platform: str) -> None:
        self.assertEqual(self.downloader.extract_urls(text), [expected_url])
        self.assertEqual(self.downloader.detect_platform(text), expected_platform)

    def test_douyin_phone_share_text(self) -> None:
        url = "https://v.douyin.com/s3eZp4vFHeU/"
        self.assert_share_text(
            "6.97 复制打开抖音，看看这个作品 " + url + " H@I.ic :1pm teb:/ 11/29",
            url,
            "douyin",
        )

    def test_kuaishou_phone_share_text(self) -> None:
        url = "https://v.kuaishou.com/AbCdEf"
        self.assert_share_text(
            "复制这条消息，打开【快手】查看精彩视频 " + url + " 该作品值得一看",
            url,
            "kuaishou",
        )

    def test_xiaohongshu_phone_share_text(self) -> None:
        url = "https://xhslink.com/a/AbCdEf"
        self.assert_share_text(
            "我在小红书发现了一篇笔记，快来看看吧！" + url + " 复制本条信息打开小红书",
            url,
            "xiaohongshu",
        )


if __name__ == "__main__":
    unittest.main()
