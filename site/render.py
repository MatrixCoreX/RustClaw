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
            "en.html", "en", "", f"{brand} by {owner} | Self-hosted AI Agent with MCP & RAG",
            f"{brand} by {owner} is a self-hosted AI agent for Raspberry Pi, Ubuntu, and macOS. Use MCP, RAG, and media skills, then review results in AiAPP.",
            "Agent chat showing the status, location, and tracking number of a sample order.",
            "mcp.png",
        ),
        (
            "zh.html", "zh-CN", "zh/", f"{brand} by {owner} | 自托管 AI Agent · MCP 工具与 RAG 知识库",
            f"{brand} 是 {owner} 开发的开源自托管 AI Agent，可运行在树莓派、Ubuntu 和 macOS，支持 MCP 工具、RAG 知识库及媒体采集与下载，可在 AiAPP 查看结果。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
            "mcp.png",
        ),
        (
            "raspberry-pi-install.html", "zh-CN", "zh/raspberry-pi-install/",
            f"在 Raspberry Pi 上安装 {brand} | {owner}",
            f"在 64 位 Raspberry Pi OS 上安装 {brand}，选择 ARM64 签名发布包，打开网页控制台并发送第一条消息。",
            "网页控制台示意，包含模拟订单查询结果。",
            "mcp.png",
        ),
        (
            "guides/skills.en.html", "en", "skills/", f"{brand} Agent Skills | Install and Use Skills",
            f"Use {brand} agent skills to collect posts, download media, and check Bitcoin prices. See how to review media results in AiAPP.",
            "The Media Discovery view shows two sample posts, their previews, and links to the sources.",
            "media-discovery.png",
        ),
        (
            "guides/skills.zh.html", "zh-CN", "zh/skills/", f"{brand} Agent 技能安装与使用指南",
            f"在 {brand} 安装并启用 Agent 技能，用自然语言采集帖子、下载媒体和查询比特币行情，并在 AiAPP 查看媒体结果。",
            "AiAPP 媒体发现展示两条采集记录、预览和来源入口。",
            "media-discovery.png",
        ),
        (
            "guides/mcp.en.html", "en", "mcp/", f"{brand} MCP Setup | Connect and Test External Tools",
            f"Connect a local stdio or remote Streamable HTTP MCP server to {brand}. Allow specific tools, test the connection, and try an order lookup.",
            "Agent chat showing the status, location, and tracking number of a sample order.",
            "mcp.png",
        ),
        (
            "guides/mcp.zh.html", "zh-CN", "zh/mcp/", f"{brand} MCP 配置指南 | 接入和测试外部工具",
            f"在 {brand} 配置本机 stdio 或远程 Streamable HTTP MCP 服务器，设置允许的工具并测试连接，再试一次订单物流查询。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
            "mcp.png",
        ),
        (
            "guides/rag.en.html", "en", "rag/", f"{brand} RAG Knowledge Base | Import Documents and Cite Sources",
            f"Import documents into a {brand} RAG knowledge base, check that the text is searchable, and ask questions with citations to the source.",
            "Agent cites a section of the sample Returns Handbook in a return shipping answer.",
            "rag.png",
        ),
        (
            "guides/rag.zh.html", "zh-CN", "zh/rag/", f"{brand} RAG 知识库指南 | 导入文档与引用出处",
            f"在 {brand} 创建 RAG 知识库，导入可读文件并验证检索。用售后手册示例学习如何指定资料库并要求 Agent 给出处。",
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
