#!/usr/bin/env bash
# 读取当前 RustClaw SQLite 数据库的结构与内容摘要（依赖系统安装 sqlite3 命令行）
# 若无 sqlite3，请使用: python scripts/db_schema_and_content.py [路径]
# 用法: scripts/db_schema_and_content.sh [数据库文件路径]
# 未指定路径时从 configs/config.toml 的 database.sqlite_path 读取，缺省为 data/rustclaw.db（相对仓库根）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONFIG="$ROOT/configs/config.toml"
DEFAULT_DB="$ROOT/data/rustclaw.db"

# 解析数据库路径
get_db_path() {
    if [[ -n "${1:-}" ]]; then
        if [[ "$1" == /* ]]; then
            echo "$1"
        else
            echo "$ROOT/$1"
        fi
        return
    fi
    if [[ -f "$CONFIG" ]]; then
        local p
        p=$(grep -E '^\s*sqlite_path\s*=' "$CONFIG" | sed -E "s/^[^=]+=\s*[\"']?([^\"']+)[\"']?/\1/" | tr -d ' ')
        if [[ -n "$p" ]]; then
            if [[ "$p" == /* ]]; then
                echo "$p"
            else
                echo "$ROOT/$p"
            fi
            return
        fi
    fi
    echo "$DEFAULT_DB"
}

DB="$(get_db_path "${1:-}")"

if [[ ! -f "$DB" ]]; then
    echo "错误: 数据库文件不存在: $DB" >&2
    echo "可指定路径: $0 [数据库文件路径]" >&2
    exit 1
fi

echo "数据库: $DB"
echo "生成时间: $(date -Iseconds)"
echo "=============================================="

# 1. 结构：sqlite_master（表/索引/视图）
echo ""
echo "========== 结构 (sqlite_master) =========="
sqlite3 "$DB" <<'SQL'
.mode line
SELECT type, name, tbl_name, sql
FROM sqlite_master
WHERE name NOT LIKE 'sqlite_%'
ORDER BY type, name;
SQL

# 2. 各表 .schema 便于阅读
echo ""
echo "========== 各表 DDL (.schema) =========="
sqlite3 "$DB" ".schema"

# 3. 内容摘要：每表行数 + 前几条样例（大字段截断）
echo ""
echo "========== 内容摘要 =========="

tables=$(sqlite3 "$DB" "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name;")
sample_rows=5

for table in $tables; do
    count=$(sqlite3 "$DB" "SELECT count(*) FROM \"$table\";")
    echo ""
    echo "--- $table (共 $count 行) ---"
    if [[ "$count" -eq 0 ]]; then
        continue
    fi
    sqlite3 -header -column "$DB" "SELECT * FROM \"$table\" LIMIT $sample_rows;"
done

echo ""
echo "=============================================="
echo "完成。完整 schema 见上方 .schema 输出；各表仅展示前 $sample_rows 行。"
