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
# - `all` 模式默认会先同步技能文档、构建 UI/dist，再交叉编译整个 workspace 并上传。
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

DEFAULT_REMOTE_USER="${REMOTE_USER:-pi}"
DEFAULT_REMOTE_HOST="${REMOTE_HOST:-192.168.31.243}"
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
SYNC_SKILL_DOCS_BEFORE_BUILD="${SYNC_SKILL_DOCS_BEFORE_BUILD:-1}"
BUILD_UI_DIST_BEFORE_UPLOAD="${BUILD_UI_DIST_BEFORE_UPLOAD:-1}"

MODE=""
ARG2=""

SSH_OPTS=()
RSYNC_SSH="ssh"
RSYNC_PROGRESS_OPTS=()

LOCAL_TARGET_RELEASE_DIR="${SCRIPT_DIR}/target/${TARGET}/${BUILD_PROFILE}"
STAGE_ROOT=""
RUSTCLAW_CARGO_METADATA_FILE=""
HOST_OS="$(detect_host_os || printf '%s' "unknown")"
HOST_ARCH="$(detect_host_arch || printf '%s' "unknown")"

log() {
	echo "[$(date '+%F %T')] $*" >&2
}

warn() {
	echo "[$(date '+%F %T')] warning: $*" >&2
}

die() {
	echo "[$(date '+%F %T')] error: $*" >&2
	exit 1
}

log_list() {
	local label="$1"
	shift || true
	local -a items=("$@")
	local item
	if [[ "${#items[@]}" -eq 0 ]]; then
		log "${label}: none"
		return 0
	fi
	log "${label} (${#items[@]}):"
	for item in "${items[@]}"; do
		echo "  - ${item}" >&2
	done
}

log_phase() {
	local title="$1"
	log "========== ${title} =========="
}

cleanup_temp_artifacts() {
	[[ -n "${STAGE_ROOT}" && -d "${STAGE_ROOT}" ]] && rm -rf "${STAGE_ROOT}"
	[[ -n "${RUSTCLAW_CARGO_METADATA_FILE}" && -f "${RUSTCLAW_CARGO_METADATA_FILE}" ]] && rm -f "${RUSTCLAW_CARGO_METADATA_FILE}"
}

trap cleanup_temp_artifacts EXIT

abs_path() {
	resolve_path_python "$1"
}

require_command() {
	local cmd="$1"
	command -v "$cmd" >/dev/null 2>&1 || die "missing command: $cmd"
}

configure_rsync_progress_opts() {
	RSYNC_PROGRESS_OPTS=()
	if [[ "${SHOW_RSYNC_PROGRESS}" == "0" ]]; then
		return 0
	fi

	if rsync --version 2>/dev/null | python3 - <<'PY'
import re
import sys

text = sys.stdin.read()
match = re.search(r"rsync\s+version\s+(\d+)\.(\d+)", text)
if not match:
    raise SystemExit(1)
major = int(match.group(1))
minor = int(match.group(2))
raise SystemExit(0 if (major, minor) >= (3, 1) else 1)
PY
	then
		RSYNC_PROGRESS_OPTS=(--info=progress2 --human-readable)
	else
		RSYNC_PROGRESS_OPTS=(--progress)
		warn "older rsync detected; progress display downgraded to --progress"
	fi
}

usage() {
	local exit_code="${1:-1}"
	cat <<EOF
Usage: $0 [options] [all|crate <package>|dir <repo-relative-dir>]

Modes:
  all             Cross-build the whole workspace locally and upload runtime files to Raspberry Pi
  crate <package> Cross-build one package locally and upload runtime files to Raspberry Pi
  dir <dir>       Upload a selected directory with common runtime/test dependencies

Options:
  --user USER          Remote username
  --host HOST          Remote host
  --key PATH           Optional SSH private key path
  --remote-dir DIR     Remote deployment directory

Common environment variables:
  REMOTE_USER/REMOTE_HOST/REMOTE_SSH_KEY/REMOTE_DIR
  BUILD_CMD               Extra local build command for dir mode
  BINARIES                Binaries to upload to remote target/release; supports spaces/newlines and shell quoting
  EXTRA_INCLUDE_PATHS     Extra repo-relative paths to upload; supports spaces/newlines and shell quoting
  SYNC_SKILL_DOCS_BEFORE_BUILD=0  Disable skill-doc sync in all mode
  BUILD_UI_DIST_BEFORE_UPLOAD=0   Disable UI/dist build in all mode
  SHOW_RSYNC_PROGRESS=0   Disable rsync progress
  SYNC_DELETE=1           Run rsync --delete on remote upload
  SKIP_UI_DIST=1          Do not build or upload UI/dist

Examples:
  $0 all
  $0 crate clawd
  BUILD_CMD='cargo build -p clawd --release --target ${TARGET}' BINARIES='clawd' $0 dir crates/clawd
EOF
	exit "$exit_code"
}

parse_args() {
	while [[ $# -gt 0 ]]; do
		case "$1" in
		--user)
			[[ $# -ge 2 ]] || die "--user requires an argument"
			REMOTE_USER="$2"
			shift 2
			;;
		--host)
			[[ $# -ge 2 ]] || die "--host requires an argument"
			REMOTE_HOST="$2"
			shift 2
			;;
		--key)
			[[ $# -ge 2 ]] || die "--key requires an argument"
			REMOTE_SSH_KEY="$2"
			shift 2
			;;
		--remote-dir)
			[[ $# -ge 2 ]] || die "--remote-dir requires an argument"
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
				[[ $# -ge 1 ]] || die "${MODE} mode requires an argument"
				ARG2="$1"
				shift
			fi
			break
			;;
		*)
			die "unknown argument: $1"
			;;
		esac
	done

	[[ -n "${MODE}" ]] || MODE="all"
	if [[ -z "${REMOTE_DIR}" ]]; then
		REMOTE_DIR="/home/${REMOTE_USER}/rustclaw"
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
	log "cargo not found locally, installing rustup..."
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	if [[ -f "${HOME}/.cargo/env" ]]; then
		# shellcheck source=/dev/null
		source "${HOME}/.cargo/env"
	fi
	command -v cargo >/dev/null 2>&1 || die "cargo installation failed; run manually: source \$HOME/.cargo/env"
}

ensure_npm() {
	if command -v npm >/dev/null 2>&1; then
		return 0
	fi
	die "npm not found; install Node.js/npm first, or set BUILD_UI_DIST_BEFORE_UPLOAD=0 / SKIP_UI_DIST=1"
}

ensure_rust_target() {
	if ! rustup target list --installed 2>/dev/null | grep -q "^${TARGET}\$"; then
		log "adding Rust target: ${TARGET}"
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
	command -v brew >/dev/null 2>&1 || die "Homebrew must be installed on macOS"
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
		command -v sudo >/dev/null 2>&1 || die "sudo is required to install dependencies on Ubuntu"
		sudo_cmd=(sudo)
	fi
	log "installing Ubuntu cross-build dependencies: ${packages[*]}"
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
			die "Xcode Command Line Tools must be installed on macOS"
		fi
		brew tap messense/macos-cross-toolchains >/dev/null 2>&1 || true
		brew_install_if_missing aarch64-unknown-linux-gnu
		brew_install_if_missing perl
		;;
	linux)
		command -v apt-get >/dev/null 2>&1 || die "automatic cross-build dependency installation currently supports only Ubuntu/Debian"
		ensure_ubuntu_packages gcc-aarch64-linux-gnu libc6-dev-arm64-cross pkg-config make perl clang libclang-dev
		;;
	*)
		die "unsupported local OS: $HOST_OS"
		;;
	esac
}

prepare_runtime_assets() {
	log_phase "1/6 Prepare runtime assets"

	if [[ "${MODE}" != "all" ]]; then
		log "mode is not all; skipping full runtime asset preparation"
		return 0
	fi

	if [[ "${SYNC_SKILL_DOCS_BEFORE_BUILD}" == "1" ]]; then
		log "syncing skill docs: scripts/sync_skill_docs.py"
		python3 "${SCRIPT_DIR}/scripts/sync_skill_docs.py"
	else
		log "skill-doc sync skipped by configuration"
	fi

	if [[ "${SKIP_UI_DIST}" == "1" ]]; then
		log "UI/dist build and upload skipped by configuration"
		return 0
	fi

	if [[ "${BUILD_UI_DIST_BEFORE_UPLOAD}" != "1" ]]; then
		log "UI/dist build skipped by configuration; using existing artifacts"
		return 0
	fi

	if [[ ! -d "${SCRIPT_DIR}/UI" ]]; then
		warn "UI directory does not exist; skipped UI/dist build"
		return 0
	fi

	ensure_npm
	log "building UI/dist: build-ui-nginx.sh --build"
	bash "${SCRIPT_DIR}/build-ui-nginx.sh" --build
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
	cross_gcc="$(detect_cross_gcc)" || die "aarch64 cross-build gcc not found; install dependencies first"
	log "cross-build toolchain: ${cross_gcc}"

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

	log "cross-build environment configured: host=${HOST_OS}/${HOST_ARCH}, target=${TARGET}, profile=${BUILD_PROFILE}"
	log "local artifact directory: ${LOCAL_TARGET_RELEASE_DIR}"
}

load_cargo_metadata() {
	if [[ -n "${RUSTCLAW_CARGO_METADATA_FILE}" && -f "${RUSTCLAW_CARGO_METADATA_FILE}" ]]; then
		return 0
	fi
	mkdir -p "${SCRIPT_DIR}/target"
	RUSTCLAW_CARGO_METADATA_FILE="$(mktemp "${SCRIPT_DIR}/target/local-cross-build-cargo-metadata.XXXXXX")"
	log "generating cargo metadata cache: ${RUSTCLAW_CARGO_METADATA_FILE}"
	cargo metadata --format-version 1 > "${RUSTCLAW_CARGO_METADATA_FILE}"
}

workspace_bins_raw() {
	load_cargo_metadata
	RUSTCLAW_CARGO_METADATA_FILE="${RUSTCLAW_CARGO_METADATA_FILE}" python3 - <<'PY'
import json
import os

with open(os.environ["RUSTCLAW_CARGO_METADATA_FILE"], "r", encoding="utf-8") as f:
    data = json.load(f)
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
	PACKAGE_NAME="$package_name" RUSTCLAW_CARGO_METADATA_FILE="${RUSTCLAW_CARGO_METADATA_FILE}" python3 - <<'PY'
import json
import os
import sys

package_name = os.environ["PACKAGE_NAME"]
with open(os.environ["RUSTCLAW_CARGO_METADATA_FILE"], "r", encoding="utf-8") as f:
    data = json.load(f)
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

print(f"package not found: {package_name}", file=sys.stderr)
sys.exit(1)
PY
}

package_workspace_dirs_raw() {
	local package_name="$1"
	load_cargo_metadata
	PACKAGE_NAME="$package_name" REPO_ROOT="${SCRIPT_DIR}" RUSTCLAW_CARGO_METADATA_FILE="${RUSTCLAW_CARGO_METADATA_FILE}" python3 - <<'PY'
import json
import os
import sys
from pathlib import Path

package_name = os.environ["PACKAGE_NAME"]
repo_root = Path(os.environ["REPO_ROOT"]).resolve()
with open(os.environ["RUSTCLAW_CARGO_METADATA_FILE"], "r", encoding="utf-8") as f:
    data = json.load(f)

workspace_members = set(data.get("workspace_members", []))
packages = {pkg["id"]: pkg for pkg in data.get("packages", [])}
nodes = {node["id"]: node for node in data.get("resolve", {}).get("nodes", [])}

start_id = None
for pkg in data.get("packages", []):
    if pkg.get("id") in workspace_members and pkg.get("name") == package_name:
        start_id = pkg["id"]
        break

if start_id is None:
    print(f"package not found: {package_name}", file=sys.stderr)
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
	DIR_REL="$repo_rel_dir" REPO_ROOT="${SCRIPT_DIR}" RUSTCLAW_CARGO_METADATA_FILE="${RUSTCLAW_CARGO_METADATA_FILE}" python3 - <<'PY'
import json
import os
import sys
from pathlib import Path

repo_root = Path(os.environ["REPO_ROOT"]).resolve()
target_dir = (repo_root / os.environ["DIR_REL"]).resolve()
with open(os.environ["RUSTCLAW_CARGO_METADATA_FILE"], "r", encoding="utf-8") as f:
    data = json.load(f)
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
		warn "skipping missing path: ${repo_rel}"
		return 0
	fi

	dest_parent="${STAGE_ROOT}/RustClaw/$(dirname "${repo_rel}")"
	mkdir -p "${dest_parent}"
	log "adding to staging: ${repo_rel}"
	rsync -a "${src}" "${dest_parent}/"
}

split_shell_words_raw() {
	local raw="${1:-}"
	[[ -n "${raw}" ]] || return 0
	LIST_RAW="${raw}" python3 - <<'PY'
import os
import shlex
import sys

raw = os.environ.get("LIST_RAW", "")
try:
    items = shlex.split(raw, posix=True)
except ValueError as exc:
    print(f"list parsing failed: {exc}", file=sys.stderr)
    raise SystemExit(1)

for item in items:
    print(item)
PY
}

copy_list_paths() {
	local raw="${1:-}"
	local paths_raw path
	paths_raw="$(split_shell_words_raw "${raw}")"
	while IFS= read -r path; do
		[[ -n "${path}" ]] || continue
		copy_repo_rel_into_stage "${path}"
	done <<< "${paths_raw}"
}

ensure_repo_relative_dir() {
	local input_path="$1"
	local abs_input repo_prefix
	abs_input="$(abs_path "${SCRIPT_DIR}/${input_path}")"
	repo_prefix="${SCRIPT_DIR}/"
	[[ -d "${abs_input}" ]] || die "directory does not exist: ${input_path}"
	if [[ "${abs_input}" != "${SCRIPT_DIR}" && "${abs_input}" != ${repo_prefix}* ]]; then
		die "dir mode only accepts directories inside the repository: ${input_path}"
	fi
}

copy_binaries_into_stage() {
	local -a binaries=("$@")
	local bin
	if [[ "${#binaries[@]}" -eq 0 ]]; then
		log "no binaries to include"
		return 0
	fi

	mkdir -p "${STAGE_ROOT}/RustClaw/target/release"
	for bin in "${binaries[@]}"; do
		[[ -n "${bin}" ]] || continue
		[[ -x "${LOCAL_TARGET_RELEASE_DIR}/${bin}" ]] || die "missing cross-build artifact: ${LOCAL_TARGET_RELEASE_DIR}/${bin}"
		log "adding binary: ${bin}"
		rsync -a "${LOCAL_TARGET_RELEASE_DIR}/${bin}" "${STAGE_ROOT}/RustClaw/target/release/${bin}"
	done
}

prepare_stage_root() {
	STAGE_ROOT="$(mktemp -d)"
	mkdir -p "${STAGE_ROOT}/RustClaw"
	log "creating staging directory: ${STAGE_ROOT}"
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
		"external_skills"
		"services/wa-web-bridge"
		"README.md"
		"USAGE.md"
		"rustclaw"
		"build-all.sh"
		"deploy-pi-nginx.sh"
		"install-rustclaw-cmd.sh"
		"start-all.sh"
		"start-all-bin.sh"
		"component_start"
		"stop-rustclaw.sh"
	)
	local path

	if [[ "${INCLUDE_RUNTIME_BASE}" != "1" ]]; then
		log "runtime base file set skipped"
		return 0
	fi

	log "adding runtime base file set"
	for path in "${paths[@]}"; do
		copy_repo_rel_into_stage "${path}"
	done

	if [[ "${SKIP_UI_DIST}" != "1" ]]; then
		if [[ -f "${SCRIPT_DIR}/UI/dist/index.html" ]]; then
			copy_repo_rel_into_stage "UI/dist"
		else
			warn "UI/dist does not exist; skipped frontend build artifacts"
		fi
	else
		log "UI/dist skipped by configuration"
	fi
}

sync_stage_to_remote() {
	local -a rsync_delete_opt=()
	if [[ "${SYNC_DELETE}" == "1" ]]; then
		rsync_delete_opt=(--delete)
	fi

	log_phase "5/6 Upload to remote"

	log "ensuring remote directory exists: ${REMOTE_DIR}"
	remote_exec "mkdir -p $(printf '%q' "${REMOTE_DIR}")"

	log "syncing staging to Raspberry Pi: ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}"
	rsync -az -e "${RSYNC_SSH}" \
		"${RSYNC_PROGRESS_OPTS[@]}" \
		"${rsync_delete_opt[@]}" \
		"${STAGE_ROOT}/RustClaw/" \
		"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"

	log_phase "6/6 Remote verification"
	log "remote directory size:"
	remote_exec "du -sh $(printf '%q' "${REMOTE_DIR}") 2>/dev/null || true"
	log "remote target/release contents:"
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
		printf -v RSYNC_SSH 'ssh -i %q' "${REMOTE_SSH_KEY}"
	fi
	configure_rsync_progress_opts

	log_phase "0/6 Task overview"
	log "deployment options: mode=${MODE}, host=${REMOTE_USER}@${REMOTE_HOST}, remote_dir=${REMOTE_DIR}"
	log "build options: target=${TARGET}, profile=${BUILD_PROFILE}, host_os=${HOST_OS}, host_arch=${HOST_ARCH}"
	log "upload options: include_runtime_base=${INCLUDE_RUNTIME_BASE}, auto_include_workspace_deps=${AUTO_INCLUDE_WORKSPACE_DEPS}, skip_ui_dist=${SKIP_UI_DIST}, sync_delete=${SYNC_DELETE}"
	log "full deployment options: sync_skill_docs_before_build=${SYNC_SKILL_DOCS_BEFORE_BUILD}, build_ui_dist_before_upload=${BUILD_UI_DIST_BEFORE_UPLOAD}"
	if [[ -n "${REMOTE_SSH_KEY}" ]]; then
		log "SSH private key: ${REMOTE_SSH_KEY}"
	fi

	prepare_runtime_assets

	case "${MODE}" in
	all)
		log_phase "2/6 Check dependencies and environment"
		ensure_local_cross_dependencies
		setup_cross_env
		log_phase "3/6 Run build"
		log "cross-building the whole workspace locally (${TARGET})..."
		cargo build --workspace --release --target "${TARGET}"
		bins_raw="$(workspace_bins_raw)"
		array_from_string_lines binaries "${bins_raw}"
		;;
	crate)
		[[ -n "${ARG2}" ]] || die "crate mode requires a package name"
		package_name="${ARG2}"
		log_phase "2/6 Check dependencies and environment"
		ensure_local_cross_dependencies
		setup_cross_env
		log_phase "3/6 Run build"
		log "cross-building package locally: ${package_name} (${TARGET})..."
		cargo build -p "${package_name}" --release --target "${TARGET}"
		bins_raw="$(package_bins_raw "${package_name}")"
		array_from_string_lines binaries "${bins_raw}"
		if [[ "${AUTO_INCLUDE_WORKSPACE_DEPS}" == "1" ]]; then
			dep_dirs_raw="$(package_workspace_dirs_raw "${package_name}")"
			array_from_string_lines dep_dirs "${dep_dirs_raw}"
		fi
		;;
	dir)
		[[ -n "${ARG2}" ]] || die "dir mode requires a repository directory"
		repo_rel_dir="${ARG2}"
		ensure_repo_relative_dir "${repo_rel_dir}"

		build_cmd="${BUILD_CMD:-}"
		if [[ -n "${build_cmd}" ]]; then
			log_phase "2/6 Check dependencies and environment"
			ensure_local_cross_dependencies
			setup_cross_env
			log_phase "3/6 Run build"
			log "running local build command: ${build_cmd}"
			(
				cd "${SCRIPT_DIR}"
				eval "${build_cmd}"
			)
		fi

		if [[ -n "${BINARIES:-}" ]]; then
			binaries_raw="$(split_shell_words_raw "${BINARIES}")"
			array_from_string_lines binaries "${binaries_raw}"
		fi

		if [[ "${AUTO_INCLUDE_WORKSPACE_DEPS}" == "1" ]]; then
			package_from_dir="$(package_name_for_dir "${repo_rel_dir}")"
			if [[ -n "${package_from_dir}" ]]; then
				log "dir mode auto-detected package: ${package_from_dir}"
				dep_dirs_raw="$(package_workspace_dirs_raw "${package_from_dir}")"
				array_from_string_lines dep_dirs "${dep_dirs_raw}"
			fi
		fi
		;;
	*)
		die "unknown mode: ${MODE}"
		;;
	esac

	log_list "binaries to upload" "${binaries[@]}"
	log_list "auto-included workspace directories" "${dep_dirs[@]}"
	if [[ -n "${repo_rel_dir}" ]]; then
		log "explicit upload directory: ${repo_rel_dir}"
	fi
	if [[ -n "${EXTRA_INCLUDE_PATHS:-}" ]]; then
		log "extra included paths: ${EXTRA_INCLUDE_PATHS}"
	fi

	log_phase "4/6 Package staging"
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
		copy_list_paths "${EXTRA_INCLUDE_PATHS}"
	fi

	copy_binaries_into_stage "${binaries[@]}"
	sync_stage_to_remote

	log "done. Remote directory: ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}"
}

main "$@"
