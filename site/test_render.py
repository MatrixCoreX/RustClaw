"""Exercise the site entry point with both repository product identity fixtures."""

from html.parser import HTMLParser
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import tomllib
import unittest
from urllib.parse import urlsplit
from xml.etree import ElementTree


SOURCE = Path(__file__).resolve().parent
FIXTURES = SOURCE.parent / "scripts/fixtures/product_identity"


class Page(HTMLParser):
    def __init__(self, source):
        super().__init__()
        self.headings = 0
        self.lang = None
        self.links = []
        self.assets = []
        self.ids = set()
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if "id" in attributes:
            self.ids.add(attributes["id"])
        if tag == "h1":
            self.headings += 1
        if tag == "html":
            self.lang = attributes["lang"]
        if tag in ("link", "a"):
            self.links.append(attributes)
        if tag == "img":
            self.assets.append(attributes["src"])


class ProjectSiteTest(unittest.TestCase):
    def test_localized_pages_and_sitemap_use_selected_identity(self):
        for fixture in ("brand-primary.toml", "brand-alternate.toml"):
            with self.subTest(fixture=fixture), TemporaryDirectory() as directory:
                output = Path(directory)
                config = FIXTURES / fixture
                identity = tomllib.loads(config.read_text(encoding="utf-8"))
                brand = identity["display_name"]
                owner, repo = identity["release_repository"].split("/")
                base = f"https://{owner.lower()}.github.io/{repo}/"
                subprocess.run(
                    [sys.executable, str(SOURCE / "render.py"), directory],
                    env=os.environ | {"APP_PRODUCT_IDENTITY_CONFIG": str(config)},
                    check=True,
                )
                for part, locale in (("", "en"), ("zh/", "zh-CN")):
                    path = output / part / "index.html"
                    source = path.read_text(encoding="utf-8")
                    page = Page(source)
                    self.assertEqual(page.lang, locale)
                    self.assertEqual(page.headings, 1)
                    self.assertIn(f"<title>{brand} by {owner} | ", source)
                    self.assertEqual([link["href"] for link in page.links if link.get("rel") == "canonical"], [base + part])
                    self.assertEqual(
                        {link["hreflang"]: link["href"] for link in page.links if link.get("rel") == "alternate"},
                        {"en": base, "zh-CN": base + "zh/", "x-default": base},
                    )
                    for link in page.links:
                        resolved = urlsplit(link["href"])
                        if not resolved.scheme and resolved.path:
                            self.assertTrue((path.parent / resolved.path).exists(), link["href"])
                        if not resolved.scheme and not resolved.path and resolved.fragment:
                            self.assertIn(resolved.fragment, page.ids)
                    for asset in page.assets:
                        self.assertGreater((path.parent / asset).stat().st_size, 0)
                sitemap = ElementTree.parse(output / "sitemap.xml")
                self.assertEqual([node.text for node in sitemap.findall(".//{*}loc")], [base, base + "zh/"])


if __name__ == "__main__":
    unittest.main()
