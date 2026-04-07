#!/usr/bin/env bash
# 在树莓派上部署现有的 UI/dist 到 nginx，并配置 RustClaw 反向代理。
# 说明:
# - 只做 deploy，不做前端构建。
# - 复用 build-ui-nginx.sh 里的 nginx 部署逻辑，避免维护两份配置。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

usage() {
	echo "用法: $0 [--path DIR]"
	echo ""
	echo "说明:"
	echo "  使用现有 UI/dist 部署 nginx，不执行 npm build。"
	echo "  默认站点根目录沿用 build-ui-nginx.sh 在 Linux 下的默认值。"
	echo ""
	echo "示例:"
	echo "  $0"
	echo "  $0 --path /var/www/html/rustclaw"
}

for arg in "$@"; do
	case "$arg" in
	--build)
		echo "错误: $0 只负责部署 nginx，不负责构建 UI。请先确保 UI/dist 已存在。" >&2
		exit 1
		;;
	-h|--help)
		usage
		exit 0
		;;
	esac
done

if [[ ! -f "${SCRIPT_DIR}/UI/dist/index.html" ]]; then
	echo "错误: 未找到 ${SCRIPT_DIR}/UI/dist/index.html" >&2
	echo "请先在其他机器构建并同步 UI/dist，或先执行 ./build-ui-nginx.sh --build" >&2
	exit 1
fi

exec bash "${SCRIPT_DIR}/build-ui-nginx.sh" --deploy "$@"
