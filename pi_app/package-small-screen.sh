#!/usr/bin/env bash
# 安装 PyInstaller 等打包依赖，并将 rustclaw_small_screen.py 打成可分发目录/单文件。
# 在 pi_app 目录下执行：./package-small-screen.sh
# 产物：dist/rustclaw-small-screen/（默认 onedir）或 dist/rustclaw-small-screen（--onefile）

set -euo pipefail

PI_APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$PI_APP_DIR"

ONEFILE=0
for arg in "$@"; do
  case "$arg" in
    --onefile) ONEFILE=1 ;;
    -h|--help)
      echo "用法: $0 [--onefile]"
      echo "  默认: onedir（树莓派上更稳，首启更快）"
      echo "  --onefile: 单文件可执行（启动时解压到临时目录，略慢）"
      exit 0
      ;;
  esac
done

VENV="${PI_APP_DIR}/.packaging-venv"
if [[ ! -d "${VENV}/bin" ]]; then
  echo "创建虚拟环境: ${VENV}"
  python3 -m venv "${VENV}"
fi
# shellcheck source=/dev/null
source "${VENV}/bin/activate"

echo "安装/升级打包依赖 (requirements-packaging.txt) ..."
pip install -q --upgrade pip
pip install -q -r "${PI_APP_DIR}/requirements-packaging.txt"

NAME="rustclaw-small-screen"
ENTRY="rustclaw_small_screen.py"

# Linux / macOS 用 ':'；Windows 为 ';'（本脚本面向 Unix）
if [[ "$(uname -s)" == MINGW* ]] || [[ "$(uname -s)" == CYGWIN_NT* ]] || [[ "$(uname -s)" == MSYS_NT* ]]; then
  DS=';'
else
  DS=':'
fi

ADD_DATA=()
add_data_tree() {
  local src="$1"
  local dest="$2"
  if [[ -e "${PI_APP_DIR}/${src}" ]]; then
    ADD_DATA+=( --add-data "${src}${DS}${dest}" )
  fi
}

add_data_tree "assets" "assets"
add_data_tree "small_screen_markets.toml" "."
add_data_tree "signature.py" "."
add_data_tree "RustClaw480X320.png" "."
add_data_tree "longxia.png" "."
add_data_tree "image" "image"

PYI=( pyinstaller --clean --noconfirm --name "${NAME}" "${ENTRY}" )
if [[ "${ONEFILE}" -eq 1 ]]; then
  PYI+=( --onefile )
else
  PYI+=( --onedir )
fi

PYI+=(
  "${ADD_DATA[@]}"
  --hidden-import=PIL.Image
  --hidden-import=PIL.ImageTk
)

echo "运行: ${PYI[*]}"
"${PYI[@]}"

if [[ "${ONEFILE}" -eq 1 ]]; then
  echo "完成: ${PI_APP_DIR}/dist/${NAME}"
else
  echo "完成: ${PI_APP_DIR}/dist/${NAME}/${NAME}"
fi
echo "说明: 语言/主题/key 仍写入可执行文件所在目录下的隐藏文件；请从仓库根运行 clawd 以保持 configs/、data/ 路径可用。"
