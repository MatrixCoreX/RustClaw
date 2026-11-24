"""Exercise the site entry point with both repository product identity fixtures."""

from html.parser import HTMLParser
import json
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import tomllib
import unittest
from urllib.parse import urljoin, urlsplit
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
        self.open_sections = []
        self.section_assets = {}
        self.ids = set()
        self.in_title = False
        self.in_structured_data = False
        self.structured_data = ""
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if "id" in attributes:
            self.ids.add(attributes["id"])
        if tag == "section":
            section_id = attributes.get("id")
            self.open_sections.append(section_id)
            if section_id:
                self.section_labels[section_id] = attributes.get("aria-labelledby")
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
        if tag == "script" and attributes.get("type") == "application/ld+json":
            self.in_structured_data = True
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
            if self.open_sections and self.open_sections[-1]:
                self.section_assets.setdefault(self.open_sections[-1], []).append(attributes["src"])

    def handle_endtag(self, tag):
        if tag == "section":
            self.open_sections.pop()
        if tag == "title":
            self.in_title = False
        if tag == "script":
            self.in_structured_data = False

    def handle_data(self, data):
        if self.in_title:
            self.title += data
        if self.in_structured_data:
            self.structured_data += data


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
                pi_guide = "raspberry-pi-install/"
                media_examples = ("media-discovery", "media-download")
                skill_examples = (*media_examples, "weather", "speech-transcription")
                example_sections = ("mcp", "rag", *skill_examples)
                topic_sections = {
                    "skills/": {"install", "aipp", "troubleshooting", *skill_examples},
                    "mcp/": {"connect", "demo", "permissions", "test", "troubleshooting"},
                    "rag/": {"prepare", "import", "ask", "test", "troubleshooting"},
                    "order-support/": {"setup", "ask", "verify", "troubleshooting"},
                    pi_guide: {"requirements", "install", "console", "troubleshooting"},
                }
                pages = [("", "en"), ("zh/", "zh-CN")]
                for topic in topic_sections:
                    pages.extend(((topic, "en"), ("zh/" + topic, "zh-CN")))
                titles = set()
                descriptions = set()
                sitemap_images = {}
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
                    canonical = base + part
                    metadata = json.loads(page.structured_data)
                    self.assertEqual(metadata["@context"], "https://schema.org")
                    if topic:
                        self.assertEqual(metadata["@type"], "BreadcrumbList")
                        trail = metadata["itemListElement"]
                        self.assertEqual([item["position"] for item in trail], [1, 2])
                        self.assertEqual(trail[0]["name"], brand)
                        self.assertEqual(trail[0]["item"], base + ("zh/" if locale == "zh-CN" else ""))
                        self.assertTrue(trail[1]["name"])
                        self.assertEqual(trail[1]["item"], canonical)
                    else:
                        repository = f"https://github.com/{owner}/{repo}"
                        self.assertEqual(metadata["@type"], "SoftwareSourceCode")
                        self.assertEqual(metadata["codeRepository"], repository)
                        self.assertEqual(metadata["name"], brand)
                        self.assertEqual(metadata["url"], base)
                        self.assertEqual(metadata["license"], repository + "/blob/main/LICENSE")
                    sitemap_images[canonical] = {
                        urljoin(canonical, src) for src in page.assets if "assets/examples/" in src
                    }
                    preview = {
                        "skills/": "media-discovery.png", "rag/": "rag.png", pi_guide: "pi-first-task.png",
                    }.get(topic, "mcp.png")
                    example_dir = f"assets/examples/{locale.split('-')[0]}/"
                    example_path = "../" * part.count("/") + example_dir
                    example_images = {section: [f"{example_path}{section}.png"] for section in example_sections}
                    for section in media_examples:
                        example_images[section].insert(0, f"{example_path}{section}-agent.png")
                    self.assertEqual(page.preview_image, base + example_dir + preview)
                    self.assertGreater((output / example_dir / preview).stat().st_size, 0)
                    alternates = {"en": base + topic, "zh-CN": base + "zh/" + topic, "x-default": base + topic}
                    if topic == pi_guide:
                        self.assertLessEqual(topic_sections[topic], page.ids)
                        self.assertIn("Raspberry Pi", page.title)
                        self.assertIn(f"install-latest-release.sh --repo {identity['release_repository']} --check-only", source)
                        self.assertIn('df -h "$HOME"', source)
                        self.assertEqual(page.section_assets["console"], [example_path + "pi-first-task.png"])
                    elif not topic:
                        self.assertTrue(page.title.startswith(brand))
                        self.assertIn(owner, page.title)
                        self.assertIn(owner, page.description)
                        self.assertLessEqual(
                            {"skills", "mcp", "rag", "media", "aipp", *skill_examples},
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
                                "pi": pi_guide,
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
                            [image for images in example_images.values() for image in images],
                        )
                        for section_id in example_sections:
                            self.assertEqual(page.section_assets[section_id], example_images[section_id])
                        hrefs = {link["href"] for link in page.links}
                        self.assertLessEqual(set(topic_sections), hrefs)
                        self.assertLessEqual({f"skills/#{name}" for name in skill_examples}, hrefs)
                    else:
                        self.assertLessEqual(topic_sections[topic], page.ids)
                        if topic == "skills/":
                            for section_id in skill_examples:
                                self.assertEqual(
                                    page.section_assets[section_id],
                                    example_images[section_id],
                                )
                        if topic == "mcp/":
                            demo = "assets/examples/mcp-orders-demo.py"
                            self.assertEqual(
                                [link["href"] for link in page.links if "download" in link],
                                ["../" * part.count("/") + demo],
                            )
                            self.assertEqual((output / demo).read_bytes(), (SOURCE / demo).read_bytes())
                            self.assertEqual(
                                page.section_assets["test"],
                                [example_path + "mcp-setup.png", example_path + "mcp.png"],
                            )
                        if topic == "rag/":
                            sample = "returns-handbook.md"
                            self.assertEqual(
                                [link["href"] for link in page.links if "download" in link],
                                [example_path + sample],
                            )
                            self.assertEqual(
                                (output / example_dir / sample).read_bytes(),
                                (SOURCE / example_dir / sample).read_bytes(),
                            )
                            self.assertEqual(page.section_assets["import"], [example_path + "rag-import.png"])
                            self.assertEqual(page.section_assets["ask"], [example_path + "rag.png"])
                        if topic == "order-support/":
                            self.assertEqual(
                                page.section_assets["verify"], [example_path + "mcp.png", example_path + "rag.png"],
                            )
                    if topic:
                        other_locale = "zh-CN" if locale == "en" else "en"
                        switches = [link for link in page.links if link.get("lang") == other_locale]
                        self.assertEqual(len(switches), 1)
                        other_path = topic if other_locale == "en" else "zh/" + topic
                        self.assertEqual((path.parent / switches[0]["href"]).resolve(), (output / other_path).resolve())
                    self.assertEqual([link["href"] for link in page.links if link.get("rel") == "canonical"], [canonical])
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
                entries = sitemap.findall("{*}url")
                self.assertCountEqual(
                    [entry.findtext("{*}loc") for entry in entries], [base + part for part, _ in pages],
                )
                image_namespace = "http://www.google.com/schemas/sitemap-image/1.1"
                for entry in entries:
                    self.assertCountEqual(
                        [node.text for node in entry.findall("image:image/image:loc", {"image": image_namespace})],
                        sitemap_images[entry.findtext("{*}loc")],
                    )
                for verification in (SOURCE / "verification").glob("google*.html"):
                    self.assertEqual((output / verification.name).read_bytes(), verification.read_bytes())

    def test_downloadable_order_server_protocol(self):
        calls = [
            ("initialize", {"protocolVersion": "2025-11-25"}),
            ("notifications/initialized", {}),
            ("ping", {}),
            ("tools/list", {}),
            ("tools/call", {"name": "lookup_order", "arguments": {"order_id": 3}}),
            ("tools/call", {"name": "find_order_by_tracking", "arguments": {"tracking_number": "7898047967"}}),
            ("tools/call", {"name": "find_order_by_tracking", "arguments": {"tracking_number": "0425386197"}}),
            ("tools/call", {"name": "lookup_order", "arguments": {"order_id": 9999}}),
            ("tools/call", {"name": "lookup_order", "arguments": {"order_id": True}}),
        ]
        requests = []
        for index, (method, params) in enumerate(calls):
            request = {"jsonrpc": "2.0", "method": method, "params": params}
            if not method.startswith("notifications/"):
                request["id"] = index
            requests.append(json.dumps(request))
        response = subprocess.run(
            [sys.executable, str(SOURCE / "assets/examples/mcp-orders-demo.py")],
            input="\n".join(requests) + "\n", check=True, capture_output=True, text=True, timeout=10,
        )
        replies = [json.loads(line) for line in response.stdout.splitlines()]
        self.assertEqual([reply["id"] for reply in replies], [0, 2, 3, 4, 5, 6, 7, 8])
        self.assertTrue(all(reply["jsonrpc"] == "2.0" for reply in replies))
        results = {reply["id"]: reply["result"] for reply in replies}
        self.assertEqual(results[0]["protocolVersion"], "2025-11-25")
        self.assertEqual(results[2], {})
        self.assertEqual([tool["name"] for tool in results[3]["tools"]], ["lookup_order", "find_order_by_tracking"])
        forward = results[4]["structuredContent"]
        self.assertEqual(forward["tracking_number"], "7898047967")
        self.assertEqual((forward["status"], forward["location"]), ("in_transit", "Hangzhou sorting center"))
        self.assertEqual(results[5]["structuredContent"], forward)
        self.assertEqual(results[6]["structuredContent"]["order_id"], 12)
        self.assertFalse(results[7]["structuredContent"]["found"])
        self.assertTrue(results[8]["isError"])


if __name__ == "__main__":
    unittest.main()
