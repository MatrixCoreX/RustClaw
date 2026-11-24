#!/usr/bin/env python3
"""Render the public project site using the repository's product identity."""

import argparse
from html import escape
from pathlib import Path
import shutil
from string import Template
import subprocess
from xml.etree.ElementTree import Element, SubElement, ElementTree


SOURCE = Path(__file__).resolve().parent
REPOSITORY_ROOT = SOURCE.parent


def read_product_identity():
    identity = subprocess.check_output(
        [
            "bash", "-c",
            r'source "$1" && printf "%s\0%s" "$APP_DISPLAY_NAME" "$APP_RELEASE_REPOSITORY"',
            "_", str(REPOSITORY_ROOT / "scripts/product_identity.sh"),
        ],
        text=True,
    )
    return identity.split("\0", 1)


def build(output: Path):
    brand, release_repository = read_product_identity()
    owner, repository = release_repository.split("/", 1)
    site = f"https://{owner.lower()}.github.io/{repository}/"
    github = f"https://github.com/{release_repository}"
    shared = {"brand": brand, "owner": owner, "site": site, "repo": github,
              "release_repository": release_repository, "profile": f"https://github.com/{owner}"}
    page = Template((SOURCE / "page.html").read_text(encoding="utf-8"))
    pages = [
        (
            "en.html", "en", "", f"{brand} | Open-Source Self-Hosted AI Agent",
            f"Self-host {brand} by {owner} on Raspberry Pi, Ubuntu, or macOS. Connect MCP tools, import RAG documents, and review media results in AiAPP.",
            "Agent chat showing the status, location, and tracking number of a sample order.",
            "mcp.png",
        ),
        (
            "zh.html", "zh-CN", "zh/", f"{brand} | 开源自托管 AI Agent",
            f"{brand} 是 {owner} 开发的开源 AI Agent，可自托管在树莓派、Ubuntu 或 macOS。接入 MCP 工具、导入 RAG 知识库，并在 AiAPP 查看媒体任务结果。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
            "mcp.png",
        ),
        (
            "raspberry-pi-install.html", "zh-CN", "zh/raspberry-pi-install/",
            f"在 Raspberry Pi 上安装 {brand} | 64 位 Raspberry Pi OS",
            f"在 64 位 Raspberry Pi OS 上安装 {brand}。检查 ARM64 架构与磁盘空间，下载签名发布包，打开网页控制台发送第一条消息。",
            "网页控制台示意，包含模拟订单查询结果。",
            "mcp.png",
        ),
        (
            "guides/skills.en.html", "en", "skills/", f"{brand} Agent Skills | Skill Store and AiAPP",
            f"Install {brand} skills from Skill Store. Explore media collection, downloads, weather forecasts, and audio transcription, with Agent and AiAPP screenshots.",
            "The Media Discovery view shows two sample posts, their previews, and links to the sources.",
            "media-discovery.png",
        ),
        (
            "guides/skills.zh.html", "zh-CN", "zh/skills/", f"{brand} 技能指南 | Skill Store 安装与 AiAPP 示例",
            f"在 {brand} 的 Skill Store 安装技能。跟着示例试用媒体采集、媒体下载、天气查询和语音转写，并在 AiAPP 查看媒体结果。",
            "AiAPP 媒体发现展示两条采集记录、预览和来源入口。",
            "media-discovery.png",
        ),
        (
            "guides/mcp.en.html", "en", "mcp/", f"{brand} MCP Setup | Local stdio and Streamable HTTP",
            f"Set up local stdio or remote Streamable HTTP in {brand}. Download a read-only Python MCP server and test order lookup by order or tracking number.",
            "Agent chat showing the status, location, and tracking number of a sample order.",
            "mcp.png",
        ),
        (
            "guides/mcp.zh.html", "zh-CN", "zh/mcp/", f"{brand} MCP 配置指南 | 本机 stdio 与远程 HTTP",
            f"在 {brand} 接入本机 stdio 或远程 Streamable HTTP MCP 服务器。下载 Python 订单示例，核对工具权限并测试订单正查和单号反查。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
            "mcp.png",
        ),
        (
            "guides/rag.en.html", "en", "rag/", f"{brand} RAG Knowledge Base Guide | Import and Cite Files",
            f"Import PDF, Markdown, or Office files into a {brand} RAG knowledge base. Download a sample handbook and verify retrieval, citations, and missing information.",
            "Agent cites a section of the sample Returns Handbook in a return shipping answer.",
            "rag.png",
        ),
        (
            "guides/rag.zh.html", "zh-CN", "zh/rag/", f"{brand} RAG 知识库指南 | 导入 PDF、Word 与 Markdown",
            f"在 {brand} 的记忆管理导入 PDF、Word 或 Markdown，检索原文并让 Agent 引用出处。配有可下载的售后手册和三道验证题。",
            "Agent 回答退货运费由谁承担，并引用示例《售后手册》第 3.2 节。",
            "rag.png",
        ),
    ]
    published_paths = {directory for _, _, directory, *_ in pages}
    urls = Element("urlset", {"xmlns": "http://www.sitemaps.org/schemas/sitemap/0.9"})
    for template, lang, directory, title, description, preview_alt, preview_file in pages:
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
            page.substitute(values, body=body, alternates=alternates), encoding="utf-8",
        )
        SubElement(SubElement(urls, "url"), "loc").text = local["canonical"]

    ElementTree(urls).write(output / "sitemap.xml", encoding="utf-8", xml_declaration=True)
    assets = output / "assets"
    assets.mkdir(exist_ok=True)
    for name in ("site.css", "console-demo.png"):
        shutil.copyfile(SOURCE / "assets" / name, assets / name)
    shutil.copytree(SOURCE / "assets" / "examples", assets / "examples", dirs_exist_ok=True)
    shutil.copyfile(REPOSITORY_ROOT / "UI/public/app-logo.svg", assets / "logo.svg")
    for verification in (SOURCE / "verification").glob("google*.html"):
        shutil.copyfile(verification, output / verification.name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    build(parser.parse_args().output)
