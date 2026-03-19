#!/usr/bin/env bash
# 统计项目代码行数（不含 target、node_modules、.git 等）
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

# 排除的目录（不统计）
EXCLUDE_DIRS="target node_modules .git UI/dist run external_skills document patches"
EXCLUDE_OPTS=()
for d in $EXCLUDE_DIRS; do
  EXCLUDE_OPTS+=( -not -path "*/${d}/*" -not -path "*/${d}" )
done

# 按类型统计：扩展名 -> 描述
count_by_ext() {
  local ext="$1"
  local desc="${2:-$ext}"
  local count
  count=$(find . -type f -name "*.${ext}" "${EXCLUDE_OPTS[@]}" 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
  printf "  %-12s %8s 行\n" "$desc" "${count:-0}"
}

echo "RustClaw 项目代码行数统计（排除 target/node_modules/.git 等）"
echo "================================================================"

# 所有参与统计的源码文件
ALL_FILES=$(mktemp)
{
  find . -type f \( -name "*.rs" -o -name "*.ts" -o -name "*.tsx" -o -name "*.js" -o -name "*.jsx" \) "${EXCLUDE_OPTS[@]}" 2>/dev/null
  find . -type f \( -name "*.py" -o -name "*.sh" \) "${EXCLUDE_OPTS[@]}" 2>/dev/null
  find . -type f \( -name "*.toml" -o -name "*.json" \) "${EXCLUDE_OPTS[@]}" 2>/dev/null
  find . -type f -name "*.md" "${EXCLUDE_OPTS[@]}" 2>/dev/null
} | sort -u > "$ALL_FILES"

total=0
if [[ -s "$ALL_FILES" ]]; then
  # 只对单文件行第一列求和（跳过 wc 的 total 行），避免 xargs 分批时合计错
  total=$(xargs wc -l < "$ALL_FILES" 2>/dev/null | awk '$NF != "total" {s+=$1} END {print s+0}')
fi
rm -f "$ALL_FILES"

echo ""
echo "按类型："
count_by_ext "rs"   "Rust"
count_by_ext "ts"   "TypeScript"
count_by_ext "tsx"  "TSX"
count_by_ext "js"   "JavaScript"
count_by_ext "jsx"  "JSX"
count_by_ext "py"   "Python"
count_by_ext "sh"   "Shell"
count_by_ext "toml" "TOML"
count_by_ext "json" "JSON"
count_by_ext "md"   "Markdown"
echo "------------------------------------------------"
printf "  %-12s %8s 行\n" "合计" "${total:-0}"
