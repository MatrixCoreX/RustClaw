#!/usr/bin/env python3
"""Derive the desktop reuse inventory from the current shared UI, without editing it."""
import hashlib
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
UI = ROOT.parent / "UI/src"
app = (UI / "App.tsx").read_text()
pages = re.findall(r'"([a-z_]+)"', re.search(r'const CONSOLE_PAGES: ConsolePage\[\] = \[([^\]]+)\]', app)[1])
lines = ["# 网页与桌面复用清单", "", "由 `python3 desktop/scripts/inventory.py` 读取当前 UI 源码生成。", "", "页面直接复用原组件，桌面请求统一经过原生 HTTPS / SSH 连接；业务服务端验收与界面可加载分开记录。", "", "| 页面 | 桌面来源 | 原生传输 | 真实业务端到端验收 |", "| --- | --- | --- | --- |"]
for page in pages:
    lines.append(f"| `{page}` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |")
lines += ["", "## 必须适配的浏览器入口", "", "| 文件 | fetch | localStorage | sessionStorage |", "| --- | ---: | ---: | ---: |"]
digest = hashlib.sha256()
for path in sorted(UI.rglob("*")):
    if not path.is_file() or path.suffix not in (".ts", ".tsx") or ".test." in path.name:
        continue
    source = path.read_text()
    digest.update(str(path.relative_to(UI)).encode()); digest.update(source.encode())
    counts = (len(re.findall(r"\bfetch\(", source)), source.count("window.localStorage"), source.count("window.sessionStorage"))
    if any(counts):
        lines.append(f"| `UI/src/{path.relative_to(UI)}` | {counts[0]} | {counts[1]} | {counts[2]} |")
lines += ["", f"源码清单摘要：`{digest.hexdigest()}`。", "", "桌面专属入口：设备添加与可信配对、HTTPS / SSH、系统凭据库、设备切换、下载保存、受限回环 Range 媒体通道、独立 AiAPP 窗口、DNS-SD 自动发现、可取消的有界 IPv4 局域网扫描。", "", "构建时适配：`scripts/shared-ui-adapter.ts` 校验已知入口出现次数；共享 UI 调整后如合同不匹配，桌面构建明确失败。", ""]
(ROOT / "docs").mkdir(exist_ok=True)
(ROOT / "docs/feature-inventory.md").write_text("\n".join(lines))
print(f"Desktop inventory: {len(pages)} shared pages")
