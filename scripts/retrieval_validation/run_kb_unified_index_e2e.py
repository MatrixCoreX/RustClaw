#!/usr/bin/env python3
"""
隔离验证 kb ingest -> unified retrieval index -> kb search。

默认行为：
- 在 scripts/retrieval_validation/.tmp_runs 下创建临时 workspace
- 复制最小 config.toml 并把 sqlite_path 指向临时数据库
- 生成测试文档
- 通过 kb-skill stdin JSON 协议执行 ingest / search
- 打印详细过程、请求、响应、数据库行摘要
- 结束后自动删除临时目录
"""

from __future__ import annotations

import argparse
import json
import os
import random
import re
import shutil
import sqlite3
import subprocess
import sys
import textwrap
from datetime import datetime
from pathlib import Path
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parents[1]
SOURCE_CONFIGS_DIR = ROOT / "configs"
SOURCE_CONFIG = SOURCE_CONFIGS_DIR / "config.toml"
TMP_ROOT = SCRIPT_DIR / ".tmp_runs"


def print_section(title: str) -> None:
    print(f"\n=== {title} ===")


def print_step(message: str) -> None:
    print(f"[STEP] {message}")


def print_info(message: str) -> None:
    print(f"[INFO] {message}")


def print_ok(message: str) -> None:
    print(f"[PASS] {message}")


def print_fail(message: str) -> None:
    print(f"[FAIL] {message}")


def pretty_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2)


def patch_sqlite_path(config_text: str, sqlite_path: str) -> str:
    patched, count = re.subn(
        r'(^\s*sqlite_path\s*=\s*)".*?"',
        rf'\1"{sqlite_path}"',
        config_text,
        count=1,
        flags=re.MULTILINE,
    )
    if count != 1:
        raise RuntimeError("未能在 config.toml 中找到 database.sqlite_path")
    return patched


def make_temp_workspace(namespace_prefix: str) -> tuple[Path, str]:
    TMP_ROOT.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    suffix = random.randint(1000, 9999)
    namespace = f"{namespace_prefix}_{stamp}_{suffix}"
    workspace = TMP_ROOT / namespace
    (workspace / "data").mkdir(parents=True, exist_ok=True)
    (workspace / "fixtures").mkdir(parents=True, exist_ok=True)
    return workspace, namespace


def write_temp_config(workspace: Path) -> Path:
    shutil.copytree(SOURCE_CONFIGS_DIR, workspace / "configs")
    raw = SOURCE_CONFIG.read_text(encoding="utf-8")
    patched = patch_sqlite_path(raw, "data/retrieval_validation.db")
    target = workspace / "configs" / "config.toml"
    target.write_text(patched, encoding="utf-8")
    return target


def write_fixture_docs(workspace: Path, namespace: str) -> list[Path]:
    docs_dir = workspace / "fixtures" / "docs"
    docs_dir.mkdir(parents=True, exist_ok=True)
    primary = docs_dir / "deploy_guide.md"
    marker = f"KB-E2E-MARKER-{namespace}"
    primary.write_text(
        textwrap.dedent(
            f"""
            # Deployment Guide

            RustClaw deployment steps for validation.

            1. Run `cargo check -p clawd`.
            2. Start services after config validation.
            3. Verify retrieval context contains the expected knowledge rows.

            Unique marker: {marker}
            """
        ).strip()
        + "\n",
        encoding="utf-8",
    )

    secondary = docs_dir / "ops_notes.md"
    secondary.write_text(
        textwrap.dedent(
            """
            # Ops Notes

            Retrieval validation should show:
            - source_kind = kb_doc
            - memory_kind = knowledge_doc
            - tool_or_skill_name = kb
            """
        ).strip()
        + "\n",
        encoding="utf-8",
    )
    return [primary, secondary]


def run_kb_skill(workspace: Path, request: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    cmd = ["cargo", "run", "--quiet", "-p", "kb-skill", "--bin", "kb-skill"]
    env = os.environ.copy()
    env["WORKSPACE_ROOT"] = str(workspace)
    print_info(f"命令: {' '.join(cmd)}")
    print_info(f"WORKSPACE_ROOT={workspace}")
    print_info("请求:")
    print(pretty_json(request))
    proc = subprocess.run(
        cmd,
        cwd=ROOT,
        env=env,
        input=json.dumps(request, ensure_ascii=False) + "\n",
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.stderr.strip():
        print_info("stderr:")
        print(proc.stderr.rstrip())
    if proc.returncode != 0:
        raise RuntimeError(
            f"kb-skill 执行失败，exit_code={proc.returncode}\nstdout={proc.stdout}\nstderr={proc.stderr}"
        )
    lines = [line for line in proc.stdout.splitlines() if line.strip()]
    if not lines:
        raise RuntimeError("kb-skill 没有输出任何 JSON")
    outer = json.loads(lines[-1])
    inner = json.loads(outer.get("text") or "{}")
    print_info("原始响应:")
    print(pretty_json(outer))
    print_info("内层 payload:")
    print(pretty_json(inner))
    return outer, inner


def db_path_for_workspace(workspace: Path) -> Path:
    return workspace / "data" / "retrieval_validation.db"


def fetch_kb_rows(db_path: Path, namespace: str) -> list[sqlite3.Row]:
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    try:
        cur = conn.cursor()
        cur.execute(
            """
            SELECT source_kind, source_ref, memory_kind, tool_or_skill_name, user_key,
                   metadata_json, substr(search_text, 1, 120) AS excerpt
            FROM memory_retrieval_index
            WHERE source_kind = 'kb_doc'
              AND source_ref LIKE ?
            ORDER BY id ASC
            """,
            (f"kb:{namespace}:%",),
        )
        return cur.fetchall()
    finally:
        conn.close()


def print_kb_rows(rows: list[sqlite3.Row]) -> None:
    if not rows:
        print_info("没有查到任何 kb_doc 行。")
        return
    for idx, row in enumerate(rows, start=1):
        print_info(f"索引行 #{idx}")
        print(
            pretty_json(
                {
                    "source_kind": row["source_kind"],
                    "source_ref": row["source_ref"],
                    "memory_kind": row["memory_kind"],
                    "tool_or_skill_name": row["tool_or_skill_name"],
                    "user_key": row["user_key"],
                    "metadata_json": json.loads(row["metadata_json"] or "{}"),
                    "excerpt": row["excerpt"],
                }
            )
        )


def validate_kb_rows(rows: list[sqlite3.Row], namespace: str) -> None:
    if not rows:
        raise RuntimeError("unified index 中没有找到 kb_doc 测试行")
    for row in rows:
        metadata = json.loads(row["metadata_json"] or "{}")
        assert row["source_kind"] == "kb_doc", row["source_kind"]
        assert row["memory_kind"] == "knowledge_doc", row["memory_kind"]
        assert row["tool_or_skill_name"] == "kb", row["tool_or_skill_name"]
        assert row["source_ref"].startswith(f"kb:{namespace}:"), row["source_ref"]
        assert metadata.get("namespace") == namespace, metadata
        assert metadata.get("path"), metadata
        assert metadata.get("chunk_id"), metadata


def validate_search_payload(payload: dict[str, Any], namespace: str) -> None:
    if payload.get("status") != "ok":
        raise RuntimeError(f"search 返回失败: {pretty_json(payload)}")
    hits = payload.get("hits") or []
    if not hits:
        raise RuntimeError("search 没有返回任何命中")
    first = hits[0]
    if namespace not in (first.get("chunk_id") or ""):
        raise RuntimeError(f"search 首条结果不属于当前 namespace: {pretty_json(first)}")


def cleanup_workspace(workspace: Path, keep_temp: bool) -> None:
    if keep_temp:
        print_info(f"--keep-temp 已开启，保留临时目录: {workspace}")
        return
    if workspace.exists():
        shutil.rmtree(workspace)
        print_ok(f"已清理临时目录: {workspace}")


def main() -> int:
    parser = argparse.ArgumentParser(description="验证 kb -> unified index -> search 链路")
    parser.add_argument(
        "--namespace-prefix",
        default="kb_e2e",
        help="测试命名空间前缀，默认 kb_e2e",
    )
    parser.add_argument(
        "--keep-temp",
        action="store_true",
        help="保留临时 workspace 便于调试",
    )
    args = parser.parse_args()

    workspace: Path | None = None
    try:
        print_section("准备临时工作区")
        workspace, namespace = make_temp_workspace(args.namespace_prefix)
        config_path = write_temp_config(workspace)
        fixture_paths = write_fixture_docs(workspace, namespace)
        print_ok(f"临时 workspace: {workspace}")
        print_info(f"配置文件: {config_path}")
        print_info(f"测试 namespace: {namespace}")
        for path in fixture_paths:
            print_info(f"测试文档: {path}")

        print_section("执行 KB Ingest")
        ingest_request = {
            "request_id": f"{namespace}-ingest",
            "args": {
                "action": "ingest",
                "namespace": namespace,
                "paths": [str(path) for path in fixture_paths],
                "file_types": ["md"],
                "overwrite": True,
                "chunk_size": 220,
                "chunk_overlap": 40,
            },
        }
        _, ingest_payload = run_kb_skill(workspace, ingest_request)
        if ingest_payload.get("status") != "ok":
            raise RuntimeError(f"ingest 失败: {pretty_json(ingest_payload)}")
        print_ok("KB ingest 成功")

        print_section("检查 Unified Index")
        db_path = db_path_for_workspace(workspace)
        print_info(f"数据库路径: {db_path}")
        rows = fetch_kb_rows(db_path, namespace)
        print_kb_rows(rows)
        validate_kb_rows(rows, namespace)
        print_ok(f"Unified index 校验通过，共 {len(rows)} 条 kb_doc 行")

        print_section("执行 KB Search")
        search_request = {
            "request_id": f"{namespace}-search",
            "args": {
                "action": "search",
                "namespace": namespace,
                "query": "deployment steps retrieval context",
                "top_k": 3,
            },
        }
        _, search_payload = run_kb_skill(workspace, search_request)
        validate_search_payload(search_payload, namespace)
        print_ok("KB search 校验通过")

        print_section("测试结论")
        print_ok("kb ingest -> unified index -> kb search 全链路验证通过")
        return 0
    except Exception as exc:
        print_section("测试失败")
        print_fail(str(exc))
        return 1
    finally:
        if workspace is not None:
            print_section("清理")
            cleanup_workspace(workspace, args.keep_temp)


if __name__ == "__main__":
    sys.exit(main())
