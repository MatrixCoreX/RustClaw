#!/usr/bin/env bash
# 在树莓派上部署现有的 UI/dist 到 nginx，并配置 RustClaw 反向代理。
# 说明:
# - 只做 deploy，不做前端构建。
# - 复用 build-ui-nginx.sh 里的 nginx 部署逻辑，避免维护两份配置。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

usage() {
	# zh: 打印树莓派 nginx 部署脚本的英文用法。
	echo "Usage: $0 [--path DIR]"
	echo ""
	echo "Description:"
	echo "  Deploy the existing UI/dist to nginx without running npm build."
	echo "  The default site root follows build-ui-nginx.sh's Linux default."
	echo ""
	echo "Examples:"
	echo "  $0"
	echo "  $0 --path /var/www/html/rustclaw"
}

for arg in "$@"; do
	case "$arg" in
	--build)
		# zh: 该脚本只部署已有 UI/dist，不负责构建前端。
		echo "Error: $0 only deploys nginx and does not build the UI. Make sure UI/dist already exists." >&2
		exit 1
		;;
	-h|--help)
		usage
		exit 0
		;;
	esac
done

if [[ ! -f "${SCRIPT_DIR}/UI/dist/index.html" ]]; then
	# zh: 缺少已构建前端产物时，提示用户先构建或同步 UI/dist。
	echo "Error: ${SCRIPT_DIR}/UI/dist/index.html not found" >&2
	echo "Build and sync UI/dist first, or run ./build-ui-nginx.sh --build" >&2
	exit 1
fi

exec bash "${SCRIPT_DIR}/build-ui-nginx.sh" --deploy "$@"
