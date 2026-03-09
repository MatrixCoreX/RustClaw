#!/usr/bin/env python3
"""
读取当前 RustClaw SQLite 数据库的结构与内容摘要。
用法: python scripts/db_schema_and_content.py [数据库文件路径]
未指定路径时从 configs/config.toml 的 database.sqlite_path 读取，缺省为 data/rustclaw.db（相对仓库根）。
输出到 stdout；若需落盘可重定向，例如: python scripts/db_schema_and_content.py > document/db_report.txt
注意：报告可能包含敏感信息（如 api_key），请勿提交到版本库或分享。
"""

import re
import sqlite3
import sys
from datetime import datetime
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parent
CONFIG = ROOT / "configs" / "config.toml"
DEFAULT_DB = ROOT / "data" / "rustclaw.db"
SAMPLE_ROWS = 5


def get_db_path(arg_path: str | None) -> Path:
    if arg_path:
        p = Path(arg_path)
        return p if p.is_absolute() else ROOT / p
    if CONFIG.exists():
        text = CONFIG.read_text(encoding="utf-8", errors="replace")
        m = re.search(r'^\s*sqlite_path\s*=\s*["\']?([^"\'\s]+)', text, re.MULTILINE)
        if m:
            p = Path(m.group(1).strip())
            return p if p.is_absolute() else ROOT / p
    return DEFAULT_DB


def main() -> None:
    db_path = get_db_path(sys.argv[1] if len(sys.argv) > 1 else None)
    if not db_path.exists():
        print(f"错误: 数据库文件不存在: {db_path}", file=sys.stderr)
        print("可指定路径: python db_schema_and_content.py [数据库文件路径]", file=sys.stderr)
        sys.exit(1)

    print(f"数据库: {db_path}")
    print(f"生成时间: {datetime.now().isoformat()}")
    print("==============================================")

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    cur = conn.cursor()

    # 1. 结构：sqlite_master
    print("\n========== 结构 (sqlite_master) ==========")
    cur.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name"
    )
    for row in cur.fetchall():
        print(f"type={row['type']} name={row['name']} tbl_name={row['tbl_name']}")
        if row["sql"]:
            print(row["sql"])
        print()

    # 2. 各表 DDL（通过 sqlite_master 的 sql 拼接）
    print("========== 各表 DDL ==========")
    cur.execute(
        "SELECT name, sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    )
    for row in cur.fetchall():
        print(row["sql"] or "")
        # 该表的索引
        cur.execute(
            "SELECT sql FROM sqlite_master WHERE type='index' AND tbl_name=? AND name NOT LIKE 'sqlite_%'",
            (row["name"],),
        )
        for idx in cur.fetchall():
            if idx["sql"]:
                print(idx["sql"])
        print()

    # 3. 内容摘要
    print("========== 内容摘要 ==========")
    cur.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    )
    tables = [r["name"] for r in cur.fetchall()]

    for table in tables:
        cur.execute(f'SELECT count(*) AS n FROM "{table}"')
        count = cur.fetchone()["n"]
        print(f"\n--- {table} (共 {count} 行) ---")
        if count == 0:
            continue
        cur.execute(f'SELECT * FROM "{table}" LIMIT {SAMPLE_ROWS}')
        rows = cur.fetchall()
        if not rows:
            continue
        keys = list(rows[0].keys())
        col_widths = {k: max(len(k), 4) for k in keys}
        for r in rows:
            for k in keys:
                col_widths[k] = max(col_widths[k], len(str(r[k])[:60]))
        fmt = "  ".join(f"{{:<{col_widths[k]}}}" for k in keys)
        print(fmt.format(*keys))
        for r in rows:
            vals = [str(r[k])[:60] for k in keys]
            print(fmt.format(*vals))

    conn.close()
    print("\n==============================================")
    print(f"完成。各表仅展示前 {SAMPLE_ROWS} 行。")


if __name__ == "__main__":
    main()
