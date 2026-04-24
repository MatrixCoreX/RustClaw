#!/usr/bin/env python3
"""
使用 configs/config.toml 中 [llm.minimax] 的配置，
对 MiniMax OpenAI 兼容接口发一条最小 chat/completions 请求，用于联通性自检。

用法:
  python3 scripts/test_minimax_connectivity.py
  python3 scripts/test_minimax_connectivity.py --config /path/to/config.toml
  python3 scripts/test_minimax_connectivity.py --model MiniMax-M2.5
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    print("需要 Python 3.11+（stdlib tomllib）", file=sys.stderr)
    sys.exit(2)

import requests

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "configs" / "config.toml"


def load_toml(path: Path) -> dict:
    with path.open("rb") as f:
        return tomllib.load(f)


def trim_slash(s: str) -> str:
    return s.rstrip("/")


def strip_think_blocks(raw: str) -> str:
    """Mirror clawd provider output cleanup for leading think blocks."""
    s = raw
    # Rust: rest.find("<think") … after_start.find("</think>")
    open_mark = "`" + "think"
    close_mark = "`" + "/think>"
    while True:
        start = s.find(open_mark)
        if start == -1:
            break
        after = s[start:]
        end_rel = after.find(close_mark)
        if end_rel == -1:
            s = s[:start]
            break
        s = s[:start] + after[end_rel + len(close_mark) :]
    return s.strip()


def strip_think_blocks_regex(raw: str) -> str:
    """再剥一层完整 `</think>`…`</think>`（不区分大小写）。"""
    s = strip_think_blocks(raw)
    _lt = chr(60)
    pat = re.compile(_lt + r"think\b[^>]*>[\s\S]*?" + _lt + r"/think>", re.IGNORECASE)
    s = pat.sub("", s)
    return s.strip()


def main() -> int:
    ap = argparse.ArgumentParser(description="MiniMax API 联通测试（读取 config.toml [llm.minimax]）")
    ap.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help=f"config.toml 路径（默认: {DEFAULT_CONFIG}）",
    )
    ap.add_argument("--model", type=str, default=None, help="覆盖配置中的 model")
    ap.add_argument(
        "--prompt",
        type=str,
        default="请只回复一个字：好",
        help="用户消息内容（默认极短以省 token）",
    )
    ap.add_argument("--max-tokens", type=int, default=32, help="max_tokens（默认 32）")
    args = ap.parse_args()

    if not args.config.is_file():
        print(f"找不到配置文件: {args.config}", file=sys.stderr)
        return 1

    cfg = load_toml(args.config)
    try:
        mm = cfg["llm"]["minimax"]
    except KeyError as e:
        print(f"配置缺少 llm.minimax 段: {e}", file=sys.stderr)
        return 1

    api_key = (mm.get("api_key") or "").strip()
    base_url = trim_slash((mm.get("base_url") or "").strip())
    model = (args.model or mm.get("model") or "").strip()
    timeout = int(mm.get("timeout_seconds") or 60)

    if not api_key or api_key == "REPLACE_ME":
        print("api_key 为空或为占位符，请在 [llm.minimax] 中填写有效 key。", file=sys.stderr)
        return 1
    if not base_url:
        print("base_url 为空。", file=sys.stderr)
        return 1
    if not model:
        print("model 为空（可用 --model 指定）。", file=sys.stderr)
        return 1

    url = f"{base_url}/chat/completions"
    body = {
        "model": model,
        "messages": [
            {"role": "user", "content": args.prompt},
        ],
        "temperature": 0,
        "max_tokens": max(1, args.max_tokens),
    }

    print(f"POST {url}")
    print(f"model={model} timeout={timeout}s")

    try:
        r = requests.post(
            url,
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
            data=json.dumps(body),
            timeout=timeout,
        )
    except requests.RequestException as e:
        print(f"请求失败: {e}", file=sys.stderr)
        return 1

    text = r.text
    if not r.ok:
        print(f"HTTP {r.status_code}", file=sys.stderr)
        # 避免把整段响应刷满屏，但保留排障信息
        snippet = text[:2000] + ("…" if len(text) > 2000 else "")
        print(snippet, file=sys.stderr)
        return 1

    try:
        data = r.json()
    except json.JSONDecodeError:
        print("响应不是合法 JSON:", file=sys.stderr)
        print(text[:2000], file=sys.stderr)
        return 1

    choices = data.get("choices")
    if not isinstance(choices, list) or not choices:
        print("响应缺少 choices:", file=sys.stderr)
        print(json.dumps(data, ensure_ascii=False, indent=2)[:4000], file=sys.stderr)
        return 1

    msg = choices[0].get("message") if isinstance(choices[0], dict) else None
    content = ""
    if isinstance(msg, dict):
        raw_c = msg.get("content")
        if isinstance(raw_c, str):
            content = strip_think_blocks_regex(raw_c)
        elif isinstance(raw_c, list):
            parts: list[str] = []
            for p in raw_c:
                if isinstance(p, dict) and p.get("type") == "text":
                    parts.append(str(p.get("text") or ""))
                elif isinstance(p, str):
                    parts.append(p)
            content = strip_think_blocks_regex("".join(parts).strip())
        else:
            content = ""

    if not content:
        print("模型返回空 content，原始 JSON（节选）:", file=sys.stderr)
        print(json.dumps(data, ensure_ascii=False, indent=2)[:4000], file=sys.stderr)
        return 1

    print("OK — 联通正常")
    print(f"回复: {content}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
