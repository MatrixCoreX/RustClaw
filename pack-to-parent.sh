#!/usr/bin/env bash
# 将当前 RustClaw 目录整体打包到上一级目录，生成 RustClaw.tar.gz（或带日期的 RustCLaw-YYYYMMDD.tar.gz）
# 用法：在仓库根目录执行 ./pack-to-parent.sh [--with-date]

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_NAME="$(basename "$SCRIPT_DIR")"
PARENT="$(dirname "$SCRIPT_DIR")"

use_date=0
for arg in "$@"; do
  case "$arg" in
    --with-date) use_date=1 ;;
    -h|--help)
      echo "Usage: $0 [--with-date]"
      echo "  Pack $ROOT_NAME/ into parent directory as ${ROOT_NAME}.tar.gz"
      echo "  --with-date  use filename ${ROOT_NAME}-YYYYMMDD.tar.gz"
      exit 0
      ;;
  esac
done

if [[ "$use_date" == "1" ]]; then
  ARCHIVE="${PARENT}/${ROOT_NAME}-$(date +%Y%m%d).tar.gz"
else
  ARCHIVE="${PARENT}/${ROOT_NAME}.tar.gz"
fi

echo "Packing $SCRIPT_DIR -> $ARCHIVE (excluding .git and target/)"
cd "$PARENT"
tar czvf "$ARCHIVE" --exclude="$ROOT_NAME/.git" --exclude="$ROOT_NAME/target" "$ROOT_NAME"
echo "Done: $ARCHIVE"
