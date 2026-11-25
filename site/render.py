#!/usr/bin/env python3
"""Render the public project site using the repository's product identity."""

import argparse
from html import escape
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import shutil
from string import Template
import subprocess
from urllib.parse import urljoin
from xml.etree.ElementTree import Element, SubElement, ElementTree, register_namespace


SOURCE = Path(__file__).resolve().parent
REPOSITORY_ROOT = SOURCE.parent
IMAGE_NAMESPACE = "http://www.google.com/schemas/sitemap-image/1.1"


class PageImages(HTMLParser):
    def __init__(self, body: str):
        super().__init__()
        self.sources: list[str] = []
        self.feed(body)

    def handle_starttag(self, tag, attrs):
        if tag == "img":
            src = dict(attrs).get("src")
            if src:
                self.sources.append(src)


def structured_data(page: dict[str, str], breadcrumb: str | None) -> str:
    data = {"@context": "https://schema.org"}
    if breadcrumb is None:
        source = {
            "@type": "SoftwareSourceCode",
            "@id": page["site"] + "#source-code",
            "name": page["brand"],
            "url": page["site"],
            "codeRepository": page["repo"],
            "programmingLanguage": "Rust",
            "license": page["repo"] + "/blob/main/LICENSE",
        }
        data["@graph"] = [
            {
                "@type": "WebSite",
                "@id": page["site"] + "#website",
                "name": page["brand"],
                "url": page["site"],
                "about": {"@id": source["@id"]},
                "inLanguage": ["en", "zh-CN"],
            },
            source,
        ]
    else:
        home = page["site"] if page["lang"] == "en" else page["site"] + "zh/"
        data |= {
            "@type": "BreadcrumbList",
            "itemListElement": [
                {"@type": "ListItem", "position": 1, "name": page["brand"], "item": home},
                {"@type": "ListItem", "position": 2, "name": breadcrumb, "item": page["canonical"]},
            ],
        }
    return json.dumps(data, ensure_ascii=False).replace("<", "\\u003c")


def read_product_identity():
    identity = subprocess.check_output(
        [
            "bash", "-c",
            r'source "$1" && printf "%s\0%s\0%s" "$APP_DISPLAY_NAME" "$APP_RELEASE_REPOSITORY" "$(product_identity_config_value public_site_url)"',
            "_", str(REPOSITORY_ROOT / "scripts/product_identity.sh"),
        ],
        text=True,
    )
    return identity.split("\0", 2)


def build(output: Path):
    brand, release_repository, public_site_url = read_product_identity()
    owner, repository = release_repository.split("/", 1)
    site = f"https://{owner.lower()}.github.io/{repository}/"
    if public_site_url:
        label = r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?"
        if not re.fullmatch(rf"https://(?:{label}\.)+{label}/?", public_site_url):
            raise ValueError("public_site_url must be the HTTPS root of a lowercase domain")
        site = public_site_url.rstrip("/") + "/"
    github = f"https://github.com/{release_repository}"
    shared = {"brand": brand, "owner": owner, "site": site, "repo": github,
              "release_repository": release_repository, "profile": f"https://github.com/{owner}"}
    page = Template((SOURCE / "page.html").read_text(encoding="utf-8"))
    pages = [
        (
            "en.html", "en", "", None, f"{brand} by {owner} | Self-Hosted AI Agent",
            f"Run {brand}, a self-hosted Rust AI agent by {owner}, on Raspberry Pi, Ubuntu, or macOS. Connect MCP tools, search a RAG knowledge base, and use AiAPP.",
            "Agent chat showing the status, location, and tracking number of a sample order.",
            "mcp.png",
        ),
        (
            "zh.html", "zh-CN", "zh/", None, f"{brand} | {owner} 的自托管 AI Agent",
            f"{brand} 是 {owner} 用 Rust 开发的自托管 AI Agent，可运行在树莓派、Ubuntu 或 macOS。接入 MCP 工具、导入 RAG 知识库，并在 AiAPP 查看媒体任务结果。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
            "mcp.png",
        ),
        (
            "guides/raspberry-pi-install.en.html", "en", "raspberry-pi-install/", "Raspberry Pi installation",
            f"Install {brand} on Raspberry Pi | 64-bit Raspberry Pi OS",
            f"Install the signed ARM64 release of {brand} on Raspberry Pi OS. Check dependencies, run the installer, and send a first message in the web console.",
            "First question and reply in the Agent web console.",
            "pi-first-task.png",
        ),
        (
            "guides/raspberry-pi-install.zh.html", "zh-CN", "zh/raspberry-pi-install/", "树莓派安装指南",
            f"在 Raspberry Pi 上安装 {brand} | 64 位 Raspberry Pi OS",
            f"在 64 位 Raspberry Pi OS 上安装 {brand}。检查 ARM64 架构与磁盘空间，下载签名发布包，打开网页控制台发送第一条消息。",
            "网页控制台中 Agent 回复第一条消息的示例。",
            "pi-first-task.png",
        ),
        (
            "guides/skills.en.html", "en", "skills/", "Agent skills", f"{brand} Agent Skills | Skill Store and AiAPP",
            f"Install {brand} skills from Skill Store. Explore media collection, downloads, weather forecasts, and audio transcription, with Agent and AiAPP screenshots.",
            "The Media Discovery view shows two sample posts, their previews, and links to the sources.",
            "media-discovery.png",
        ),
        (
            "guides/skills.zh.html", "zh-CN", "zh/skills/", "Agent 技能", f"{brand} 技能指南 | Skill Store 安装与 AiAPP 示例",
            f"在 {brand} 的 Skill Store 安装技能。跟着示例试用媒体采集、媒体下载、天气查询和语音转写，并在 AiAPP 查看媒体结果。",
            "AiAPP 媒体发现展示两条采集记录、预览和来源入口。",
            "media-discovery.png",
        ),
        (
            "guides/mcp.en.html", "en", "mcp/", "MCP setup", f"{brand} MCP Setup | Local stdio and Streamable HTTP",
            f"Set up local stdio or remote Streamable HTTP in {brand}. Download a read-only Python MCP server and test order lookup by order or tracking number.",
            "Agent chat showing the status, location, and tracking number of a sample order.",
            "mcp.png",
        ),
        (
            "guides/mcp.zh.html", "zh-CN", "zh/mcp/", "MCP 配置", f"{brand} MCP 配置指南 | 本机 stdio 与远程 HTTP",
            f"在 {brand} 接入本机 stdio 或远程 Streamable HTTP MCP 服务器。下载 Python 订单示例，核对工具权限并测试订单正查和单号反查。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
            "mcp.png",
        ),
        (
            "guides/rag.en.html", "en", "rag/", "RAG knowledge base", f"{brand} RAG Knowledge Base Guide | Import and Cite Files",
            f"Import PDF, Markdown, or Office files into a {brand} RAG knowledge base. Download seven examples covering product questions, warehouse procedures, and MCP order lookup.",
            "Agent cites a section of the sample Returns Handbook in a return shipping answer.",
            "rag.png",
        ),
        (
            "guides/rag.zh.html", "zh-CN", "zh/rag/", "RAG 知识库", f"{brand} RAG 知识库指南 | 导入 PDF、Word 与 Markdown",
            f"在 {brand} 的记忆管理导入 PDF、Word 或 Markdown 并核对引用。下载七个 RAG 案例，体验商品导购、发货流程、文档对比和 MCP 订单查询。",
            "Agent 回答退货运费由谁承担，并引用示例《售后手册》第 3.2 节。",
            "rag.png",
        ),
        (
            "guides/order-support.en.html", "en", "order-support/", "Order support workflow",
            f"{brand} Order Support Workflow | MCP and RAG",
            f"Use {brand} to look up an order through MCP and cite a returns policy from RAG in one reply. Follow the steps with a sample server and handbook.",
            "Agent shows the shipping location and tracking number of the sample order.",
            "mcp.png",
        ),
        (
            "guides/order-support.zh.html", "zh-CN", "zh/order-support/", "订单售后实战",
            f"{brand} 订单售后实战 | MCP 查物流与 RAG 查规则",
            f"用 {brand} 的 MCP 工具查订单物流，用 RAG 知识库查退货规则。结合模拟订单和演示手册，核对同一条回答里的快递单号和政策出处。",
            "Agent 展示模拟订单的物流位置和快递单号。",
            "mcp.png",
        ),
    ]
    published_paths = {directory for _, _, directory, *_ in pages}
    register_namespace("image", IMAGE_NAMESPACE)
    urls = Element("urlset", {"xmlns": "http://www.sitemaps.org/schemas/sitemap/0.9"})
    for template, lang, directory, breadcrumb, title, description, preview_alt, preview_file in pages:
        assets_path = "../" * directory.count("/") + "assets/"
        language = lang.split("-", 1)[0]
        example_path = f"examples/{language}/"
        en_path = directory.removeprefix("zh/")
        zh_path = "zh/" + en_path
        alternates = ""
        if {en_path, zh_path} <= published_paths:
            alternates = "\n".join(
                f'  <link rel="alternate" hreflang="{locale}" href="{escape(site + path, quote=True)}">'
                for locale, path in (("en", en_path), ("zh-CN", zh_path), ("x-default", en_path))
            )
        local = shared | {
            "lang": lang,
            "title": title,
            "canonical": site + directory,
            "description": description,
            "preview_alt": preview_alt,
            "preview_image": site + "assets/" + example_path + preview_file,
            "assets": assets_path,
            "examples": assets_path + example_path,
        }
        values = {key: escape(value, quote=True) for key, value in local.items()}
        body = Template((SOURCE / template).read_text(encoding="utf-8")).substitute(values)
        if template.startswith("guides/"):
            layout = Template((SOURCE / f"guide-{language}.html").read_text(encoding="utf-8"))
            language_switch = "../" + zh_path if language == "en" else "../../" + en_path
            body = layout.substitute(values, guide_body=body, language_switch=escape(language_switch, quote=True))
        target = output / directory
        target.mkdir(parents=True, exist_ok=True)
        (target / "index.html").write_text(
            page.substitute(
                values, body=body, alternates=alternates, structured_data=structured_data(local, breadcrumb),
            ),
            encoding="utf-8",
        )
        entry = SubElement(urls, "url")
        SubElement(entry, "loc").text = local["canonical"]
        images = (urljoin(local["canonical"], src) for src in PageImages(body).sources)
        for url in dict.fromkeys(images):
            if url.startswith(site + "assets/examples/"):
                image = SubElement(entry, f"{{{IMAGE_NAMESPACE}}}image")
                SubElement(image, f"{{{IMAGE_NAMESPACE}}}loc").text = url

    ElementTree(urls).write(output / "sitemap.xml", encoding="utf-8", xml_declaration=True)
    (output / "robots.txt").write_text(f"User-agent: *\nAllow: /\nSitemap: {site}sitemap.xml\n", encoding="utf-8")
    assets = output / "assets"
    assets.mkdir(exist_ok=True)
    for name in ("site.css", "console-demo.png", "favicon.png"):
        shutil.copyfile(SOURCE / "assets" / name, assets / name)
    examples = assets / "examples"
    shutil.copytree(SOURCE / "assets" / "examples", examples, dirs_exist_ok=True)
    shutil.make_archive(str(examples / "rag-scenarios"), "zip", root_dir=examples, base_dir="rag-scenarios")
    shutil.copyfile(REPOSITORY_ROOT / "UI/public/app-logo.svg", assets / "logo.svg")
    for verification in (SOURCE / "verification").glob("google*.html"):
        shutil.copyfile(verification, output / verification.name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    build(parser.parse_args().output)
