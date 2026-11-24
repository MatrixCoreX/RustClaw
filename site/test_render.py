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
        self.section_labels = {}
        self.section_headings = set()
        self.platform_options = []
        self.platform_labels = set()
        self.lang = None
        self.description = ""
        self.title = ""
        self.preview_image = ""
        self.links = []
        self.assets = []
        self.ids = set()
        self.in_title = False
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if "id" in attributes:
            self.ids.add(attributes["id"])
        if tag == "section" and "id" in attributes:
            self.section_labels[attributes["id"]] = attributes.get("aria-labelledby")
        if tag == "h2" and "id" in attributes:
            self.section_headings.add(attributes["id"])
        if tag == "input" and attributes.get("name") == "install-platform":
            self.platform_options.append(attributes)
        if tag == "label" and "for" in attributes:
            self.platform_labels.add(attributes["for"])
        if tag == "h1":
            self.headings += 1
        if tag == "title":
            self.in_title = True
        if tag == "html":
            self.lang = attributes["lang"]
        if tag == "meta" and attributes.get("name") == "description":
            self.description = attributes["content"]
        if tag == "meta" and attributes.get("property") == "og:image":
            self.preview_image = attributes["content"]
        if tag in ("link", "a"):
            self.links.append(attributes)
        if tag == "img":
            self.assets.append(attributes["src"])

    def handle_endtag(self, tag):
        if tag == "title":
            self.in_title = False

    def handle_data(self, data):
        if self.in_title:
            self.title += data


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
                pi_guide = "zh/raspberry-pi-install/"
                topic_sections = {
                    "skills/": {"install", "media-discovery", "media-download", "crypto", "aipp", "troubleshooting"},
                    "mcp/": {"connect", "permissions", "test", "troubleshooting"},
                    "rag/": {"prepare", "import", "ask", "troubleshooting"},
                }
                pages = [("", "en"), ("zh/", "zh-CN"), (pi_guide, "zh-CN")]
                for topic in topic_sections:
                    pages.extend(((topic, "en"), ("zh/" + topic, "zh-CN")))
                titles = set()
                descriptions = set()
                for part, locale in pages:
                    path = output / part / "index.html"
                    source = path.read_text(encoding="utf-8")
                    page = Page(source)
                    self.assertEqual(page.lang, locale)
                    self.assertEqual(page.headings, 1)
                    self.assertIn(brand, page.title)
                    self.assertTrue(page.description)
                    self.assertNotIn(page.title, titles)
                    self.assertNotIn(page.description, descriptions)
                    titles.add(page.title)
                    descriptions.add(page.description)
                    topic = part.removeprefix("zh/")
                    preview = {"skills/": "media-discovery.png", "rag/": "rag.png"}.get(topic, "mcp.png")
                    example_dir = f"assets/examples/{locale.split('-')[0]}/"
                    self.assertEqual(page.preview_image, base + example_dir + preview)
                    self.assertGreater((output / example_dir / preview).stat().st_size, 0)
                    alternates = {"en": base + topic, "zh-CN": base + "zh/" + topic, "x-default": base + topic}
                    if part == pi_guide:
                        self.assertIn(f"<title>在 Raspberry Pi 上安装 {brand} | {owner}</title>", source)
                        self.assertIn(f"install-latest-release.sh --repo {identity['release_repository']} --check-only", source)
                        self.assertIn('df -h "$HOME"', source)
                        alternates = {}
                    elif not topic:
                        self.assertIn(f"<title>{brand} by {owner} | ", source)
                        self.assertLessEqual(
                            {"skills", "mcp", "rag", "media", "media-discovery", "media-download", "crypto", "aipp"},
                            page.ids,
                        )
                        for section_id in ("skills", "mcp", "rag"):
                            self.assertIn(page.section_labels[section_id], page.section_headings)
                        option_ids = [option["id"] for option in page.platform_options]
                        self.assertEqual(option_ids, ["install-pi", "install-ubuntu", "install-macos"])
                        self.assertEqual(page.platform_labels, set(option_ids))
                        self.assertTrue(all(option["type"] == "radio" for option in page.platform_options))
                        self.assertEqual(
                            [option["id"] for option in page.platform_options if "checked" in option],
                            ["install-pi"],
                        )
                        doc = "release_installation.md" if locale == "en" else "release_installation.zh-CN.md"
                        guide_url = f"https://github.com/{owner}/{repo}/blob/main/docs/{doc}"
                        self.assertEqual(
                            {link["data-platform"]: link["href"] for link in page.links if "data-platform" in link},
                            {
                                "pi": pi_guide if locale == "en" else "raspberry-pi-install/",
                                "ubuntu": guide_url + "#ubuntu",
                                "macos": guide_url + "#macos",
                            },
                        )
                        documentation = (SOURCE.parent / "docs" / doc).read_text(encoding="utf-8")
                        self.assertIn("\n### Ubuntu\n", documentation)
                        self.assertIn("\n### macOS\n", documentation)
                        self.assertIn("MCP", page.description)
                        self.assertIn("RAG", page.description)
                        self.assertIn("AiAPP", page.description)
                        self.assertCountEqual(
                            [asset for asset in page.assets if "assets/examples/" in asset],
                            ["../" * part.count("/") + example_dir + name for name in (
                                "mcp.png", "rag.png", "media-discovery.png", "media-download.png", "crypto.png",
                            )],
                        )
                        hrefs = {link["href"] for link in page.links}
                        self.assertLessEqual(set(topic_sections), hrefs)
                        self.assertIn("skills/#crypto", hrefs)
                        self.assertIn(pi_guide if not part else "raspberry-pi-install/", hrefs)
                    else:
                        self.assertLessEqual(topic_sections[topic], page.ids)
                        if topic == "skills/":
                            self.assertIn("../" * part.count("/") + example_dir + "crypto.png", page.assets)
                        other_locale = "zh-CN" if locale == "en" else "en"
                        switches = [link for link in page.links if link.get("lang") == other_locale]
                        self.assertEqual(len(switches), 1)
                        other_path = topic if other_locale == "en" else "zh/" + topic
                        self.assertEqual((path.parent / switches[0]["href"]).resolve(), (output / other_path).resolve())
                    self.assertEqual([link["href"] for link in page.links if link.get("rel") == "canonical"], [base + part])
                    self.assertEqual(
                        {link["hreflang"]: link["href"] for link in page.links if link.get("rel") == "alternate"},
                        alternates,
                    )
                    for link in page.links:
                        resolved = urlsplit(link["href"])
                        if resolved.scheme or resolved.netloc:
                            continue
                        target = path.parent / resolved.path if resolved.path else path
                        if target.is_dir():
                            target /= "index.html"
                        self.assertTrue(target.is_file(), link["href"])
                        if resolved.fragment:
                            linked_page = page if not resolved.path else Page(target.read_text(encoding="utf-8"))
                            self.assertIn(resolved.fragment, linked_page.ids, link["href"])
                    for asset in page.assets:
                        self.assertGreater((path.parent / asset).stat().st_size, 0)
                sitemap = ElementTree.parse(output / "sitemap.xml")
                self.assertCountEqual([node.text for node in sitemap.findall(".//{*}loc")], [base + part for part, _ in pages])
                for verification in (SOURCE / "verification").glob("google*.html"):
                    self.assertEqual((output / verification.name).read_bytes(), verification.read_bytes())


if __name__ == "__main__":
    unittest.main()
