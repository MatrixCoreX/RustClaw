#!/usr/bin/env bash
# 本机交叉编译 aarch64 Linux 产物，并把运行/测试所需文件同步到远端树莓派。
# 用法:
#   ./local-cross-build-upload-pi.sh all
#   ./local-cross-build-upload-pi.sh crate <package>
#   ./local-cross-build-upload-pi.sh dir <repo-relative-dir>
#
# 说明:
# - 本脚本在本机编译，兼容 macOS 和 Ubuntu。
# - 编译产物来自 target/aarch64-unknown-linux-gnu/release/，上传时会放到远端 target/release/。
# - `dir` 模式会把指定目录完整同步到远端，并额外附带 RustClaw 运行/测试常用依赖文件。
# - `dir` 模式可结合环境变量:
#     BUILD_CMD='cargo build -p clawd --release --target aarch64-unknown-linux-gnu'
#     BINARIES='clawd telegramd'
#     EXTRA_INCLUDE_PATHS='crates/claw-core crates/clawd'
# - 可通过环境变量覆盖远端信息:
#     REMOTE_USER REMOTE_HOST REMOTE_SSH_KEY REMOTE_DIR

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
cd "${SCRIPT_DIR}"

if [[ -f "${HOME}/.cargo/env" ]]; then
	# shellcheck source=/dev/null
	source "${HOME}/.cargo/env"
fi

DEFAULT_REMOTE_USER="${REMOTE_USER:-testuser}"
DEFAULT_REMOTE_HOST="${REMOTE_HOST:-192.168.31.162}"
DEFAULT_REMOTE_SSH_KEY="${REMOTE_SSH_KEY:-}"

REMOTE_USER="${DEFAULT_REMOTE_USER}"
REMOTE_HOST="${DEFAULT_REMOTE_HOST}"
REMOTE_SSH_KEY="${DEFAULT_REMOTE_SSH_KEY}"
REMOTE_DIR=""
TARGET="${TARGET:-aarch64-unknown-linux-gnu}"
BUILD_PROFILE="${BUILD_PROFILE:-release}"
SHOW_RSYNC_PROGRESS="${SHOW_RSYNC_PROGRESS:-1}"
SYNC_DELETE="${SYNC_DELETE:-0}"
SKIP_UI_DIST="${SKIP_UI_DIST:-0}"
INCLUDE_RUNTIME_BASE="${INCLUDE_RUNTIME_BASE:-1}"
AUTO_INCLUDE_WORKSPACE_DEPS="${AUTO_INCLUDE_WORKSPACE_DEPS:-1}"

MODE=""
ARG2=""

SSH_OPTS=()
RSYNC_SSH="ssh"
RSYNC_PROGRESS_OPTS=()
if [[ "${SHOW_RSYNC_PROGRESS}" != "0" ]]; then
	RSYNC_PROGRESS_OPTS=(--info=progress2 --human-readable)
fi

LOCAL_TARGET_RELEASE_DIR="${SCRIPT_DIR}/target/${TARGET}/${BUILD_PROFILE}"
STAGE_ROOT=""
RUSTCLAW_CARGO_METADATA=""
HOST_OS="$(detect_host_os || printf '%s' "unknown")"
HOST_ARCH="$(detect_host_arch || printf '%s' "unknown")"

log() {
	echo "[$(date '+%F %T')] $*"
}

warn() {
	echo "[$(date '+%F %T')] warning: $*" >&2
}

die() {
	echo "[$(date '+%F %T')] error: $*" >&2
	exit 1
}

abs_path() {
	resolve_path_python "$1"
}

require_command() {
	local cmd="$1"
	command -v "$cmd" >/dev/null 2>&1 || die "缺少命令: $cmd"
}

usage() {
	local exit_code="${1:-1}"
	cat <<EOF
用法: $0 [选项] [all|crate <package>|dir <repo-relative-dir>]

模式:
  all             本机交叉编译整个 workspace，并上传运行目录到树莓派
  crate <package> 本机交叉编译指定 package，并上传运行目录到树莓派
  dir <dir>       上传指定目录，并附带运行/测试常用依赖文件

命令行选项:
  --user USER          远端用户名
  --host HOST          远端地址
  --key PATH           可选，手动指定 SSH 私钥路径
  --remote-dir DIR     远端部署目录

常用环境变量:
  REMOTE_USER/REMOTE_HOST/REMOTE_SSH_KEY/REMOTE_DIR
  BUILD_CMD               dir 模式下，本机额外执行的构建命令
  BINARIES                需一并上传到远端 target/release 的二进制，空格分隔
  EXTRA_INCLUDE_PATHS     额外上传的 repo 相对路径，空格分隔
  SHOW_RSYNC_PROGRESS=0   关闭 rsync 进度
  SYNC_DELETE=1           上传时对远端执行 rsync --delete
  SKIP_UI_DIST=1          不上传 UI/dist

示例:
  $0 --user pi --host 192.168.31.50 --remote-dir /home/pi/rustclaw_runtime all
  $0 --user pi --host 192.168.31.50 crate clawd
  BUILD_CMD='cargo build -p clawd --release --target ${TARGET}' BINARIES='clawd' $0 --host 192.168.31.50 dir crates/clawd
EOF
	exit "$exit_code"
}

parse_args() {
	while [[ $# -gt 0 ]]; do
		case "$1" in
		--user)
			[[ $# -ge 2 ]] || die "--user 需要参数"
			REMOTE_USER="$2"
			shift 2
			;;
		--host)
			[[ $# -ge 2 ]] || die "--host 需要参数"
			REMOTE_HOST="$2"
			shift 2
			;;
		--key)
			[[ $# -ge 2 ]] || die "--key 需要参数"
			REMOTE_SSH_KEY="$2"
			shift 2
			;;
		--remote-dir)
			[[ $# -ge 2 ]] || die "--remote-dir 需要参数"
			REMOTE_DIR="$2"
			shift 2
			;;
		-h|--help)
			usage 0
			;;
		all|crate|dir)
			MODE="$1"
			shift
			if [[ "${MODE}" == "crate" || "${MODE}" == "dir" ]]; then
				[[ $# -ge 1 ]] || die "${MODE} 模式缺少参数"
				ARG2="$1"
				shift
			fi
			break
			;;
		*)
			die "未知参数: $1"
			;;
		esac
	done

	[[ -n "${MODE}" ]] || MODE="all"
	if [[ -z "${REMOTE_DIR}" ]]; then
		REMOTE_DIR="/home/${REMOTE_USER}/rustclaw_runtime"
	fi
}

remote_exec() {
	local command="$1"
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" \
		"bash -lc $(printf '%q' "$command")"
}

ensure_cargo() {
	if command -v cargo >/dev/null 2>&1; then
		return 0
	fi
	log "本机未检测到 cargo，正在安装 rustup..."
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	if [[ -f "${HOME}/.cargo/env" ]]; then
		# shellcheck source=/dev/null
		source "${HOME}/.cargo/env"
	fi
	command -v cargo >/dev/null 2>&1 || die "cargo 安装失败，请手动执行: source \$HOME/.cargo/env"
}

ensure_rust_target() {
	if ! rustup target list --installed 2>/dev/null | grep -q "^${TARGET}\$"; then
		log "添加 Rust target: ${TARGET}"
		rustup target add "${TARGET}"
	fi
}

ensure_homebrew() {
	if command -v brew >/dev/null 2>&1; then
		return 0
	fi
	if [[ -x /opt/homebrew/bin/brew ]]; then
		eval "$(/opt/homebrew/bin/brew shellenv)"
	elif [[ -x /usr/local/bin/brew ]]; then
		eval "$(/usr/local/bin/brew shellenv)"
	fi
	command -v brew >/dev/null 2>&1 || die "macOS 需要先安装 Homebrew"
}

brew_install_if_missing() {
	local formula="$1"
	if brew list --versions "$formula" >/dev/null 2>&1; then
		return 0
	fi
	log "brew install ${formula}"
	brew install "$formula"
}

ensure_ubuntu_packages() {
	local -a packages=("$@")
	local sudo_cmd=()
	if [[ "${EUID}" -ne 0 ]]; then
		command -v sudo >/dev/null 2>&1 || die "Ubuntu 安装依赖需要 sudo"
		sudo_cmd=(sudo)
	fi
	log "安装 Ubuntu 交叉编译依赖: ${packages[*]}"
	"${sudo_cmd[@]}" apt-get update -qq
	"${sudo_cmd[@]}" apt-get install -y -qq "${packages[@]}"
}

ensure_local_cross_dependencies() {
	require_command ssh
	require_command rsync
	require_command python3
	require_command curl

	ensure_cargo
	ensure_rust_target

	case "$HOST_OS" in
	macos)
		ensure_homebrew
		if ! xcode-select -p >/dev/null 2>&1; then
			die "macOS 需要先安装 Xcode Command Line Tools"
		fi
		brew tap messense/macos-cross-toolchains >/dev/null 2>&1 || true
		brew_install_if_missing aarch64-unknown-linux-gnu
		brew_install_if_missing perl
		;;
	linux)
		command -v apt-get >/dev/null 2>&1 || die "当前仅自动支持 Ubuntu/Debian 安装交叉编译依赖"
		ensure_ubuntu_packages gcc-aarch64-linux-gnu libc6-dev-arm64-cross pkg-config make perl clang libclang-dev
		;;
	*)
		die "不支持的本机系统: $HOST_OS"
		;;
	esac
}

detect_cross_gcc() {
	local gcc_path toolchain_prefix

	if [[ "$HOST_OS" == "macos" ]] && command -v brew >/dev/null 2>&1; then
		toolchain_prefix="$(brew --prefix aarch64-unknown-linux-gnu 2>/dev/null || true)"
		if [[ -n "$toolchain_prefix" && -d "$toolchain_prefix/bin" ]]; then
			export PATH="${toolchain_prefix}/bin:${PATH}"
		fi
	fi

	for gcc_path in aarch64-linux-gnu-gcc aarch64-unknown-linux-gnu-gcc; do
		if command -v "$gcc_path" >/dev/null 2>&1; then
			command -v "$gcc_path"
			return 0
		fi
	done
	return 1
}

setup_cross_env() {
	local cross_gcc cross_bin_dir cross_bin_prefix gcc_include_dir gcc_sysroot target_include_dir extra_args
	cross_gcc="$(detect_cross_gcc)" || die "未找到 aarch64 交叉编译 gcc，请先安装依赖"

	export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="${cross_gcc}"
	export CC_aarch64_unknown_linux_gnu="${cross_gcc}"
	export PKG_CONFIG_ALLOW_CROSS=1

	cross_bin_dir="$(dirname "${cross_gcc}")"
	cross_bin_prefix="$(basename "${cross_gcc}")"
	cross_bin_prefix="${cross_bin_prefix%gcc}"

	if [[ -x "${cross_bin_dir}/${cross_bin_prefix}g++" ]]; then
		export CXX_aarch64_unknown_linux_gnu="${cross_bin_dir}/${cross_bin_prefix}g++"
	fi
	if [[ -x "${cross_bin_dir}/${cross_bin_prefix}ar" ]]; then
		export AR_aarch64_unknown_linux_gnu="${cross_bin_dir}/${cross_bin_prefix}ar"
	fi

	gcc_include_dir="$("${cross_gcc}" -print-file-name=include 2>/dev/null || true)"
	gcc_sysroot="$("${cross_gcc}" -print-sysroot 2>/dev/null || true)"
	target_include_dir=""
	for target_include_dir in \
		"${gcc_sysroot}/usr/include" \
		"${gcc_sysroot}/include" \
		"/usr/aarch64-linux-gnu/include"; do
		if [[ -n "${target_include_dir}" && -d "${target_include_dir}" ]]; then
			break
		fi
	done

	if [[ -n "${gcc_include_dir}" && -d "${gcc_include_dir}" ]]; then
		extra_args="--target=aarch64-linux-gnu -I${gcc_include_dir}"
		if [[ -n "${target_include_dir}" && -d "${target_include_dir}" ]]; then
			extra_args="${extra_args} -I${target_include_dir}"
		fi
		export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_gnu="${extra_args}"
	fi
}

load_cargo_metadata() {
	if [[ -n "${RUSTCLAW_CARGO_METADATA}" ]]; then
		return 0
	fi
	RUSTCLAW_CARGO_METADATA="$(cargo metadata --format-version 1)"
	export RUSTCLAW_CARGO_METADATA
}

workspace_bins_raw() {
	load_cargo_metadata
	python3 - <<'PY'
import json
import os

data = json.loads(os.environ["RUSTCLAW_CARGO_METADATA"])
workspace_members = set(data.get("workspace_members", []))
bins = set()

for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    for target in pkg.get("targets", []):
        if "bin" in target.get("kind", []):
            name = (target.get("name") or "").strip()
            if name:
                bins.add(name)

for name in sorted(bins):
    print(name)
PY
}

package_bins_raw() {
	local package_name="$1"
	load_cargo_metadata
	PACKAGE_NAME="$package_name" python3 - <<'PY'
import json
import os
import sys

package_name = os.environ["PACKAGE_NAME"]
data = json.loads(os.environ["RUSTCLAW_CARGO_METADATA"])
workspace_members = set(data.get("workspace_members", []))

for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    if pkg.get("name") != package_name:
        continue
    bins = []
    for target in pkg.get("targets", []):
        if "bin" in target.get("kind", []):
            name = (target.get("name") or "").strip()
            if name:
                bins.append(name)
    for name in sorted(set(bins)):
        print(name)
    sys.exit(0)

print(f"未找到 package: {package_name}", file=sys.stderr)
sys.exit(1)
PY
}

package_workspace_dirs_raw() {
	local package_name="$1"
	load_cargo_metadata
	PACKAGE_NAME="$package_name" REPO_ROOT="${SCRIPT_DIR}" python3 - <<'PY'
import json
import os
import sys
from pathlib import Path

package_name = os.environ["PACKAGE_NAME"]
repo_root = Path(os.environ["REPO_ROOT"]).resolve()
data = json.loads(os.environ["RUSTCLAW_CARGO_METADATA"])

workspace_members = set(data.get("workspace_members", []))
packages = {pkg["id"]: pkg for pkg in data.get("packages", [])}
nodes = {node["id"]: node for node in data.get("resolve", {}).get("nodes", [])}

start_id = None
for pkg in data.get("packages", []):
    if pkg.get("id") in workspace_members and pkg.get("name") == package_name:
        start_id = pkg["id"]
        break

if start_id is None:
    print(f"未找到 package: {package_name}", file=sys.stderr)
    sys.exit(1)

stack = [start_id]
seen = set()
dirs = []
while stack:
    current = stack.pop()
    if current in seen:
        continue
    seen.add(current)
    pkg = packages.get(current)
    if not pkg:
        continue

    manifest_dir = Path(pkg["manifest_path"]).resolve().parent
    try:
        rel = manifest_dir.relative_to(repo_root)
    except ValueError:
        rel = None
    if rel is not None:
        dirs.append(str(rel))

    node = nodes.get(current, {})
    for dep in node.get("deps", []):
        dep_id = dep.get("pkg")
        if dep_id in workspace_members and dep_id not in seen:
            stack.append(dep_id)

for entry in sorted(set(dirs)):
    print(entry)
PY
}

package_name_for_dir() {
	local repo_rel_dir="$1"
	load_cargo_metadata
	DIR_REL="$repo_rel_dir" REPO_ROOT="${SCRIPT_DIR}" python3 - <<'PY'
import json
import os
import sys
from pathlib import Path

repo_root = Path(os.environ["REPO_ROOT"]).resolve()
target_dir = (repo_root / os.environ["DIR_REL"]).resolve()
data = json.loads(os.environ["RUSTCLAW_CARGO_METADATA"])
workspace_members = set(data.get("workspace_members", []))

best_name = ""
best_len = -1
for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    pkg_dir = Path(pkg["manifest_path"]).resolve().parent
    try:
        target_dir.relative_to(pkg_dir)
    except ValueError:
        continue
    current_len = len(str(pkg_dir))
    if current_len > best_len:
        best_name = pkg.get("name", "")
        best_len = current_len

if best_name:
    print(best_name)
PY
}

copy_repo_rel_into_stage() {
	local repo_rel="$1"
	local src="${SCRIPT_DIR}/${repo_rel}"
	local dest_parent

	if [[ ! -e "${src}" ]]; then
		warn "跳过缺失路径: ${repo_rel}"
		return 0
	fi

	dest_parent="${STAGE_ROOT}/RustClaw/$(dirname "${repo_rel}")"
	mkdir -p "${dest_parent}"
	rsync -a "${src}" "${dest_parent}/"
}

copy_space_separated_paths() {
	local raw="${1:-}"
	local path
	for path in ${raw}; do
		[[ -n "${path}" ]] || continue
		copy_repo_rel_into_stage "${path}"
	done
}

ensure_repo_relative_dir() {
	local input_path="$1"
	local abs_input repo_prefix
	abs_input="$(abs_path "${SCRIPT_DIR}/${input_path}")"
	repo_prefix="${SCRIPT_DIR}/"
	[[ -d "${abs_input}" ]] || die "目录不存在: ${input_path}"
	if [[ "${abs_input}" != "${SCRIPT_DIR}" && "${abs_input}" != ${repo_prefix}* ]]; then
		die "dir 模式只接受仓库内目录: ${input_path}"
	fi
}

copy_binaries_into_stage() {
	local -a binaries=("$@")
	local bin
	if [[ "${#binaries[@]}" -eq 0 ]]; then
		return 0
	fi

	mkdir -p "${STAGE_ROOT}/RustClaw/target/release"
	for bin in "${binaries[@]}"; do
		[[ -n "${bin}" ]] || continue
		[[ -x "${LOCAL_TARGET_RELEASE_DIR}/${bin}" ]] || die "缺少交叉编译产物: ${LOCAL_TARGET_RELEASE_DIR}/${bin}"
		rsync -a "${LOCAL_TARGET_RELEASE_DIR}/${bin}" "${STAGE_ROOT}/RustClaw/target/release/${bin}"
	done
}

prepare_stage_root() {
	STAGE_ROOT="$(mktemp -d)"
	trap '[[ -n "${STAGE_ROOT}" ]] && rm -rf "${STAGE_ROOT}"' EXIT
	mkdir -p "${STAGE_ROOT}/RustClaw"
}

copy_runtime_base_paths() {
	local -a paths=(
		"Cargo.toml"
		"Cargo.lock"
		".cargo"
		"configs"
		"prompts"
		"migrations"
		"scripts"
		"pi_app"
		"services/wa-web-bridge"
		"README.md"
		"USAGE.md"
		"rustclaw"
		"build-all.sh"
		"install-rustclaw-cmd.sh"
		"start-all.sh"
		"start-all-bin.sh"
		"start-clawd.sh"
		"start-clawd-ui.sh"
		"start-telegramd.sh"
		"start-wechatd.sh"
		"start-whatsappd.sh"
		"start-whatsapp-webd.sh"
		"start-future-adapters.sh"
		"stop-rustclaw.sh"
	)
	local path

	if [[ "${INCLUDE_RUNTIME_BASE}" != "1" ]]; then
		return 0
	fi

	for path in "${paths[@]}"; do
		copy_repo_rel_into_stage "${path}"
	done

	if [[ "${SKIP_UI_DIST}" != "1" ]]; then
		if [[ -f "${SCRIPT_DIR}/UI/dist/index.html" ]]; then
			copy_repo_rel_into_stage "UI/dist"
		else
			warn "UI/dist 不存在，已跳过前端构建产物"
		fi
	fi
}

sync_stage_to_remote() {
	local -a rsync_delete_opt=()
	if [[ "${SYNC_DELETE}" == "1" ]]; then
		rsync_delete_opt=(--delete)
	fi

	log "确保远端目录存在: ${REMOTE_DIR}"
	remote_exec "mkdir -p $(printf '%q' "${REMOTE_DIR}")"

	log "同步 staging 到树莓派: ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}"
	rsync -az -e "${RSYNC_SSH}" \
		"${RSYNC_PROGRESS_OPTS[@]}" \
		"${rsync_delete_opt[@]}" \
		"${STAGE_ROOT}/RustClaw/" \
		"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"

	log "远端目录大小:"
	remote_exec "du -sh $(printf '%q' "${REMOTE_DIR}") 2>/dev/null || true"
	log "远端 target/release 内容:"
	remote_exec "ls -lh $(printf '%q' "${REMOTE_DIR}/target/release") 2>/dev/null || true"
}

main() {
	local package_name="" repo_rel_dir="" build_cmd=""
	local binaries_raw bins_raw dep_dirs_raw package_from_dir
	local -a binaries=()
	local -a dep_dirs=()
	local dep_dir

	parse_args "$@"

	SSH_OPTS=()
	RSYNC_SSH="ssh"
	if [[ -n "${REMOTE_SSH_KEY}" ]]; then
		SSH_OPTS=(-i "${REMOTE_SSH_KEY}")
		RSYNC_SSH="ssh -i ${REMOTE_SSH_KEY}"
	fi
	RSYNC_PROGRESS_OPTS=()
	if [[ "${SHOW_RSYNC_PROGRESS}" != "0" ]]; then
		RSYNC_PROGRESS_OPTS=(--info=progress2 --human-readable)
	fi

	case "${MODE}" in
	all)
		ensure_local_cross_dependencies
		setup_cross_env
		log "本机交叉编译整个 workspace (${TARGET})..."
		cargo build --workspace --release --target "${TARGET}"
		bins_raw="$(workspace_bins_raw)"
		array_from_string_lines binaries "${bins_raw}"
		;;
	crate)
		[[ -n "${ARG2}" ]] || die "crate 模式必须指定 package 名"
		package_name="${ARG2}"
		ensure_local_cross_dependencies
		setup_cross_env
		log "本机交叉编译 package: ${package_name} (${TARGET})..."
		cargo build -p "${package_name}" --release --target "${TARGET}"
		bins_raw="$(package_bins_raw "${package_name}")"
		array_from_string_lines binaries "${bins_raw}"
		if [[ "${AUTO_INCLUDE_WORKSPACE_DEPS}" == "1" ]]; then
			dep_dirs_raw="$(package_workspace_dirs_raw "${package_name}")"
			array_from_string_lines dep_dirs "${dep_dirs_raw}"
		fi
		;;
	dir)
		[[ -n "${ARG2}" ]] || die "dir 模式必须指定仓库内目录"
		repo_rel_dir="${ARG2}"
		ensure_repo_relative_dir "${repo_rel_dir}"

		build_cmd="${BUILD_CMD:-}"
		if [[ -n "${build_cmd}" ]]; then
			ensure_local_cross_dependencies
			setup_cross_env
			log "执行本机构建命令: ${build_cmd}"
			(
				cd "${SCRIPT_DIR}"
				eval "${build_cmd}"
			)
		fi

		if [[ -n "${BINARIES:-}" ]]; then
			binaries_raw="$(printf '%s\n' ${BINARIES})"
			array_from_string_lines binaries "${binaries_raw}"
		fi

		if [[ "${AUTO_INCLUDE_WORKSPACE_DEPS}" == "1" ]]; then
			package_from_dir="$(package_name_for_dir "${repo_rel_dir}")"
			if [[ -n "${package_from_dir}" ]]; then
				dep_dirs_raw="$(package_workspace_dirs_raw "${package_from_dir}")"
				array_from_string_lines dep_dirs "${dep_dirs_raw}"
			fi
		fi
		;;
	*)
		die "未知模式: ${MODE}"
		;;
	esac

	prepare_stage_root
	copy_runtime_base_paths

	if [[ -n "${repo_rel_dir}" ]]; then
		copy_repo_rel_into_stage "${repo_rel_dir}"
	fi

	for dep_dir in "${dep_dirs[@]}"; do
		[[ -n "${dep_dir}" ]] || continue
		copy_repo_rel_into_stage "${dep_dir}"
	done

	if [[ -n "${EXTRA_INCLUDE_PATHS:-}" ]]; then
		copy_space_separated_paths "${EXTRA_INCLUDE_PATHS}"
	fi

	copy_binaries_into_stage "${binaries[@]}"
	sync_stage_to_remote

	log "完成。远端目录: ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}"
}

main "$@"
