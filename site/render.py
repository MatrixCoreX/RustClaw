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
        ),
        (
            "zh.html", "zh-CN", "zh/", f"{brand} by {owner} | 自托管 AI Agent · MCP 工具与 RAG 知识库",
            f"{brand} 是 {owner} 开发的开源自托管 AI Agent，可运行在树莓派、Ubuntu 和 macOS，支持 MCP 工具、RAG 知识库及媒体采集与下载，可在 AiAPP 查看结果。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
        ),
        (
            "raspberry-pi-install.html", "zh-CN", "zh/raspberry-pi-install/",
            f"在 Raspberry Pi 上安装 {brand} | {owner}",
            f"在 64 位 Raspberry Pi OS 上安装 {brand}，选择 ARM64 签名发布包，打开网页控制台并发送第一条消息。",
            "网页控制台示意，包含模拟订单查询结果。",
        ),
    ]
    home_alternates = "\n".join(
        f'  <link rel="alternate" hreflang="{language}" href="{escape(site + path, quote=True)}">'
        for language, path in (("en", ""), ("zh-CN", "zh/"), ("x-default", ""))
    )
    urls = Element("urlset", {"xmlns": "http://www.sitemaps.org/schemas/sitemap/0.9"})
    for template, lang, directory, title, description, preview_alt in pages:
        assets_path = "../" * directory.count("/") + "assets/"
        example_path = f"examples/{lang.split('-')[0]}/"
        local = shared | {
            "lang": lang,
            "title": title,
            "canonical": site + directory,
            "description": description,
            "preview_alt": preview_alt,
            "preview_image": site + "assets/" + example_path + "mcp.png",
            "assets": assets_path,
            "examples": assets_path + example_path,
        }
        values = {key: escape(value, quote=True) for key, value in local.items()}
        body = Template((SOURCE / template).read_text(encoding="utf-8"))
        target = output / directory
        target.mkdir(parents=True, exist_ok=True)
        alternates = home_alternates if directory in {"", "zh/"} else ""
        (target / "index.html").write_text(
            page.substitute(values, body=body.substitute(values), alternates=alternates), encoding="utf-8",
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
