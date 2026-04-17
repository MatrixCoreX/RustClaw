#!/usr/bin/env bash
# 检查 configs 等文件中是否包含敏感信息（api_key、token、secret、app_secret 等），
# 避免被提交进仓库。Git 没有「add 时」的钩子，因此防护在 commit 时生效；
# 安装 pre-commit 后，执行 git commit 会自动检查暂存区，含敏感信息会拒绝提交。
#
# 用法:
#   ./scripts/check-secrets.sh              # 检查已暂存文件
#   ./scripts/check-secrets.sh --staged     # 同上
#   ./scripts/check-secrets.sh configs/     # 检查 configs/ 下所有文件
#   ./scripts/check-secrets.sh --install    # 安装 pre-commit 钩子（commit 时自动检查）
# 退出码: 0=未发现敏感信息, 1=发现敏感信息

set -e

# 视为占位符或安全的值（不报错）
SAFE_REGEX='REPLACE_ME|your_.*_here|\$\{[A-Z_]+\}|^["'\''"]?["'\''"]\s*$|\*\*\*'

# 检测单行是否像“真实密钥”
check_line() {
  local file="$1" line="$2" num="$3"
  # 跳过注释、节名
  [[ "$line" =~ ^[[:space:]]*# ]] && return 0
  [[ "$line" =~ ^[[:space:]]*\[ ]] && return 0
  # 含安全占位符则通过
  echo "$line" | grep -qE "$SAFE_REGEX" && return 0
  # 疑似真实密钥：= "sk-..." 或 = "长字符串"
  if echo "$line" | grep -qE '(api_key|token|secret|password|app_secret)[[:space:]]*=[[:space:]]*"[^"]+"'; then
    if echo "$line" | grep -qE '=[[:space:]]*"sk-[^"]+"'; then
      echo "SECRET: $file:$num: $line"
      return 1
    fi
    if echo "$line" | grep -qE '=[[:space:]]*"[a-zA-Z0-9_-]{24,}"'; then
      echo "SECRET: $file:$num: $line"
      return 1
    fi
  fi
  return 0
}

check_file() {
  local f="$1"
  [[ ! -f "$f" ]] && return 0
  # 只检查 configs 下或常见配置扩展名
  if [[ "$f" != configs/* ]] && [[ "$f" != *.toml ]] && [[ "$f" != *.yaml ]] && [[ "$f" != *.yml ]] && [[ "$f" != *.env ]]; then
    return 0
  fi
  local n=0
  while IFS= read -r line; do
    ((n++)) || true
    check_line "$f" "$line" "$n" || FAILED=1
  done < "$f"
  return 0
}

install_hook() {
  local hook_dir hook_path script_dir
  script_dir="$(cd "$(dirname "$0")" && pwd)"
  hook_dir="$(git rev-parse --git-dir)/hooks"
  hook_path="$hook_dir/pre-commit"
  mkdir -p "$hook_dir"
  root="$(git rev-parse --show-toplevel)"
  cat > "$hook_path" << HOOK
#!/usr/bin/env bash
# Pre-commit: 禁止把 configs 下的敏感信息提交进仓库
exec "$root/scripts/check-secrets.sh" --staged
HOOK
  chmod +x "$hook_path"
  echo "已安装 pre-commit 钩子: $hook_path"
  echo "之后执行 git commit 时会自动检查暂存区中的 configs 等文件是否含敏感信息。"
  return 0
}

MODE=""
FILES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --staged)   MODE=staged; shift ;;
    --install)  install_hook; exit 0 ;;
    --help|-h)
      echo "Usage: $0 [--staged] [--install] [PATH...]"
      echo "  --staged   Check only staged files (default if no PATH)."
      echo "  --install  Install git pre-commit hook to run this check on commit."
      echo "  PATH       Files or dirs to check (default: staged files)."
      exit 0
      ;;
    *) FILES+=("$1"); shift ;;
  esac
done

FAILED=0

if [[ ${#FILES[@]} -eq 0 ]] || [[ "$MODE" == staged ]]; then
  STAGED=$(git diff --cached --name-only --diff-filter=ACMR 2>/dev/null || true)
  if [[ -z "$STAGED" ]]; then
    if [[ "$MODE" == staged ]]; then
      echo "No staged files to check."
      exit 0
    fi
    FILES=(configs)
  else
    while IFS= read -r f; do
      [[ -z "$f" ]] && continue
      check_file "$f" || FAILED=1
    done <<< "$STAGED"
  fi
fi

for path in "${FILES[@]}"; do
  if [[ -d "$path" ]]; then
    while IFS= read -r f; do
      check_file "$f" || FAILED=1
    done < <(find "$path" -type f 2>/dev/null)
  else
    check_file "$path" || FAILED=1
  fi
done

if [[ $FAILED -eq 1 ]]; then
  echo ""
  echo "请勿提交上述敏感信息。可将敏感项改为占位符（如 REPLACE_ME_xxx）或从暂存区移除："
  echo "  git reset HEAD -- <文件>"
  exit 1
fi
exit 0
