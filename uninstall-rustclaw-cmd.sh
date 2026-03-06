#!/usr/bin/env bash
set -euo pipefail

# 仅卸载 rustclaw 命令（删除安装目录下的 rustclaw 链接或可执行文件），不修改、不删除任何配置或项目文件。

DEFAULT_INSTALL_DIR="/usr/local/bin"
USER_INSTALL_DIR="${HOME}/.local/bin"
USE_USER_DIR=0
INSTALL_DIR="$DEFAULT_INSTALL_DIR"

usage() {
  cat <<'EOF'
Usage:
  bash uninstall-rustclaw-cmd.sh [options]

Options:
  --user       Uninstall from ~/.local/bin (same as install --user)
  --dir <path> Uninstall from custom directory (same as install --dir)
  -h, --help   Show this help

Only removes the rustclaw command from the chosen directory. Does not touch
configs, data, logs, or any project files.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --user)
      USE_USER_DIR=1
      INSTALL_DIR="$USER_INSTALL_DIR"
      ;;
    --dir)
      shift
      if [[ $# -lt 1 ]]; then
        echo "Missing value for --dir"
        exit 1
      fi
      INSTALL_DIR="$1"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
  shift
done

LINK_PATH="$INSTALL_DIR/rustclaw"

if [[ ! -e "$LINK_PATH" ]]; then
  echo "Nothing to uninstall: $LINK_PATH does not exist."
  exit 0
fi

if [[ -w "$INSTALL_DIR" ]]; then
  rm -f "$LINK_PATH"
  echo "Uninstalled: $LINK_PATH removed."
elif command -v sudo >/dev/null 2>&1; then
  sudo rm -f "$LINK_PATH"
  echo "Uninstalled: $LINK_PATH removed."
else
  echo "No write permission to $INSTALL_DIR and sudo is unavailable. Cannot remove $LINK_PATH."
  exit 1
fi
