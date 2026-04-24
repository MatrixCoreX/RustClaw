#!/usr/bin/env bash
set -euo pipefail

# Flow:
# 1) backup local configs/
# 2) sanitize configs/**/*.toml (redact secrets + channels enabled=false)
# 3) git add configs && git commit && git push
# 4) restore local configs/ exactly as before
# zh: 将脱敏后的 configs 提交并推送；本地真实配置会先备份，结束后原样恢复。

usage() {
# zh: 打印命令用法；运行时保持英文输出。
  cat <<'EOF'
Usage:
  push-sanitized-configs.sh -m "commit message" [-r origin] [-b branch]

Options:
  -m  Commit message (required)
  -r  Remote name (default: origin)
  -b  Branch name (default: current branch)
  -h  Show help
EOF
}

MSG=""
REMOTE="origin"
BRANCH=""

while getopts ":m:r:b:h" opt; do
  case "${opt}" in
    m) MSG="${OPTARG}" ;;
    r) REMOTE="${OPTARG}" ;;
    b) BRANCH="${OPTARG}" ;;
    h)
      usage
      exit 0
      ;;
    \?)
      echo "Unknown option: -${OPTARG}" >&2
      usage
      exit 1
      ;;
    :)
      echo "Option -${OPTARG} requires an argument." >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "${MSG}" ]]; then
  echo "Commit message is required: -m \"...\"" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${REPO_ROOT}" ]]; then
  echo "Cannot locate git repository root from: ${SCRIPT_DIR}" >&2
  exit 1
fi
cd "${REPO_ROOT}"

if [[ -z "${BRANCH}" ]]; then
  BRANCH="$(git rev-parse --abbrev-ref HEAD)"
fi

CONFIG_DIR="${REPO_ROOT}/configs"
if [[ ! -d "${CONFIG_DIR}" ]]; then
  echo "configs/ directory not found: ${CONFIG_DIR}" >&2
  exit 1
fi

BACKUP_DIR="$(mktemp -d)"
RESTORED=0

restore_configs() {
  if [[ ${RESTORED} -eq 1 ]]; then
    return
  fi
  RESTORED=1
  if [[ -d "${BACKUP_DIR}/configs" ]]; then
    rsync -a --delete "${BACKUP_DIR}/configs/" "${CONFIG_DIR}/"
    echo "[restore] local configs restored."
  fi
  rm -rf "${BACKUP_DIR}"
}

trap restore_configs EXIT

echo "[backup] saving local configs/..."
rsync -a --delete "${CONFIG_DIR}/" "${BACKUP_DIR}/configs/"

echo "[sanitize] redacting secrets in configs/**/*.toml ..."
python3 - "${CONFIG_DIR}" <<'PY'
import re
import sys
from pathlib import Path

config_dir = Path(sys.argv[1])

SENSITIVE_EXACT = {
    "bot_token",
    "access_token",
    "api_key",
    "api_keys",
    "api_secret",
    "app_secret",
    "client_secret",
    "private_key",
    "verify_token",
    "verification_token",
    "encrypt_key",
    "password",
    "passphrase",
    "user_key",
}

def is_sensitive_key(key: str) -> bool:
    k = key.strip().lower()
    if k in SENSITIVE_EXACT:
        return True
    return (
        k.endswith("_key")
        or k.endswith("_keys")
        or k.endswith("_token")
        or k.endswith("_tokens")
        or k.endswith("_secret")
        or k.endswith("_secrets")
        or k.endswith("_password")
        or k.endswith("_passphrase")
    )

assign_re = re.compile(
    r'(?P<key>[A-Za-z0-9_.-]+)\s*=\s*(?P<quote>"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\')',
    re.IGNORECASE,
)

enabled_re = re.compile(
    r'^(?P<prefix>\s*enabled\s*=\s*)(?P<val>true|false)(?P<tail>\s*(?:#.*)?)$',
    re.IGNORECASE,
)

for path in sorted(config_dir.rglob("*.toml")):
    text = path.read_text(encoding="utf-8")
    original = text

    def repl(m):
        key = m.group("key")
        if not is_sensitive_key(key):
            return m.group(0)
        return f'{key} = ""'

    text = assign_re.sub(repl, text)

    rel = path.relative_to(config_dir).as_posix()
    if rel.startswith("channels/"):
        lines = text.splitlines(keepends=True)
        out = []
        for line in lines:
            m = enabled_re.match(line.rstrip("\n"))
            if m and m.group("val").lower() == "true":
                line = f'{m.group("prefix")}false{m.group("tail")}\n'
            out.append(line)
        text = "".join(out)

    if text != original:
        path.write_text(text, encoding="utf-8")
        print(f"[changed] {path}")
PY

echo "[git] add sanitized configs ..."
git add configs

if git diff --cached --quiet; then
  echo "[info] nothing staged after sanitization. skip commit."
else
  echo "[git] commit ..."
  git commit -m "${MSG}"
fi

echo "[git] push ${REMOTE} ${BRANCH} ..."
git push "${REMOTE}" "HEAD:${BRANCH}"
echo "[done] push completed."
