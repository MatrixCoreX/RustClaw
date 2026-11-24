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
    shared = {"brand": brand, "owner": owner, "site": site, "repo": github, "profile": f"https://github.com/{owner}"}
    page = Template((SOURCE / "page.html").read_text(encoding="utf-8"))
    locales = [
        (
            "en", "", "Self-hosted AI Agent",
            f"{brand} by {owner} is a self-hosted AI agent for Raspberry Pi, Ubuntu, and macOS, with MCP tools, RAG knowledge bases, and a web console.",
            "Agent chat showing the status, location, and tracking number of a sample order.",
        ),
        (
            "zh-CN", "zh/", "自托管 AI Agent",
            f"{brand} 是 {owner} 开发的自托管 AI Agent，支持 MCP 工具、RAG 知识库与网页控制台，可部署在树莓派、Ubuntu 和 macOS 上。",
            "Agent 对话窗口展示模拟订单的状态、物流位置和快递单号。",
        ),
    ]
    urls = Element("urlset", {"xmlns": "http://www.sitemaps.org/schemas/sitemap/0.9"})
    for lang, directory, tagline, description, preview_alt in locales:
        local = shared | {
            "lang": lang,
            "title": f"{brand} by {owner} | {tagline}",
            "canonical": site + directory,
            "description": description,
            "preview_alt": preview_alt,
            "assets": "../assets/" if directory else "assets/",
        }
        values = {key: escape(value, quote=True) for key, value in local.items()}
        body = Template((SOURCE / f"{lang.split('-')[0]}.html").read_text(encoding="utf-8"))
        target = output / directory
        target.mkdir(parents=True, exist_ok=True)
        (target / "index.html").write_text(page.substitute(values, body=body.substitute(values)), encoding="utf-8")
        SubElement(SubElement(urls, "url"), "loc").text = local["canonical"]

    ElementTree(urls).write(output / "sitemap.xml", encoding="utf-8", xml_declaration=True)
    assets = output / "assets"
    assets.mkdir(exist_ok=True)
    for name in ("site.css", "console-demo.png"):
        shutil.copyfile(SOURCE / "assets" / name, assets / name)
    shutil.copyfile(REPOSITORY_ROOT / "UI/public/app-logo.svg", assets / "logo.svg")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    build(parser.parse_args().output)
