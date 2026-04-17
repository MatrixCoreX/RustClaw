#!/usr/bin/env python3
"""
验证 knowledge_fact -> semantic_fact -> RELEVANT_FACTS 召回链路。

该脚本直接运行专用 Rust 单元测试，并展示：
- 测试步骤
- cargo test 命令
- 测试内打印的 inserted row / recalled facts / memory block
- 清理说明（本测试使用内存 SQLite，无磁盘残留）
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parents[1]
TEST_NAME = "memory::retrieval::tests::knowledge_fact_rows_recall_into_relevant_facts"


def print_section(title: str) -> None:
    print(f"\n=== {title} ===")


def print_info(message: str) -> None:
    print(f"[INFO] {message}")


def print_ok(message: str) -> None:
    print(f"[PASS] {message}")


def print_fail(message: str) -> None:
    print(f"[FAIL] {message}")


def main() -> int:
    cmd = ["cargo", "test", "-p", "clawd", TEST_NAME, "--", "--nocapture"]

    print_section("准备")
    print_info("该验证使用 clawd 内部专用单元测试")
    print_info("数据库使用 SQLite in-memory，测试结束后不会留下磁盘数据")

    print_section("执行测试")
    print_info(f"工作目录: {ROOT}")
    print_info(f"命令: {' '.join(cmd)}")
    proc = subprocess.run(
        cmd,
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    if proc.stdout.strip():
        print(proc.stdout.rstrip())
    if proc.stderr.strip():
        print_section("stderr")
        print(proc.stderr.rstrip())

    print_section("清理")
    print_ok("本测试使用内存 SQLite，无需额外删除临时数据库")

    if proc.returncode != 0:
        print_section("结论")
        print_fail(f"knowledge_fact 召回验证失败，exit_code={proc.returncode}")
        return proc.returncode

    print_section("结论")
    print_ok("knowledge_fact -> semantic_fact -> RELEVANT_FACTS 验证通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
