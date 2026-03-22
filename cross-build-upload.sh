#!/usr/bin/env bash
# 上传 RustClaw 到高配机，远程交叉编译 aarch64，结束后取回
# 用法: ./cross-build-upload.sh [all|skill <name>|crate <name>|dir]
#  dir 模式用环境变量指定上传/拉回：UPLOAD_PATHS BUILD_CMD PULL_REMOTE PULL_LOCAL
# 依赖: 远程为 Linux，脚本会自动检测并安装 rustup/target/gcc-aarch64-linux-gnu

set -e
SKIP_REMOTE_ENV="${SKIP_REMOTE_ENV:-}"
CROSS_PULL_ALL_ARTIFACTS="${CROSS_PULL_ALL_ARTIFACTS:-}"
CLEAN_REMOTE_TMP_FIRST="${CLEAN_REMOTE_TMP_FIRST:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_HOST="${REMOTE_HOST:-45.77.104.123}"
if [[ -z "${REMOTE_SSH_KEY}" ]]; then
	if [[ -f "${HOME}/.ssh/id_ed25519" ]]; then
		REMOTE_SSH_KEY="${HOME}/.ssh/id_ed25519"
	else
		REMOTE_SSH_KEY="${HOME}/.ssh/id_rsa"
	fi
fi
REMOTE_DIR="/tmp/rustclaw-cross-new"
LOCAL_SOURCE="${SCRIPT_DIR}"
LOCAL_OUTPUT="${SCRIPT_DIR}"
TARGET="aarch64-unknown-linux-gnu"
LOCAL_RELEASE_DIR="${LOCAL_OUTPUT}/target/release"

abs_path() { echo "$(cd "$(dirname "$1")" 2>/dev/null && pwd)/$(basename "$1")" || echo "$1"; }
format_mib() { awk -v bytes="${1:-0}" 'BEGIN { printf "%.2f", bytes / 1048576 }'; }

SSH_OPTS=(-i "${REMOTE_SSH_KEY}")
RSYNC_SSH="ssh -i ${REMOTE_SSH_KEY}"
REMOTE_CARGO_ENV='source ~/.cargo/env 2>/dev/null; export PATH="$HOME/.cargo/bin:$PATH"; '
# bindgen 在 aarch64 交叉编译时需要显式看到目标头文件，否则 silk-rs 会报 float.h not found
REMOTE_BINDGEN_ENV='GCC_INCLUDE_DIR="$(aarch64-linux-gnu-gcc -print-file-name=include 2>/dev/null)"; TARGET_INCLUDE_DIR="/usr/aarch64-linux-gnu/include"; if [[ -n "$GCC_INCLUDE_DIR" && -d "$TARGET_INCLUDE_DIR" ]]; then export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_gnu="--target=aarch64-linux-gnu -I$GCC_INCLUDE_DIR -I$TARGET_INCLUDE_DIR"; fi; '

# 统计并打印拉回产物大小（目录或单个文件）
print_pull_stats() {
	local dest="$1"
	local label="${2:-拉回}"
	if [[ ! -e "$dest" ]]; then
		echo "[$(date)] ${label}: 路径不存在 $dest"
		return
	fi
	if [[ -d "$dest" ]]; then
		local total
		total=$(du -sh "$dest" 2>/dev/null | cut -f1)
		echo "[$(date)] ${label} 总大小: ${total}"
		echo "[$(date)] 文件列表（不含 deps/build/*.rlib/*.d）:"
		find "$dest" -maxdepth 1 -type f \( ! -name '*.rlib' ! -name '*.d' \) -exec ls -lh {} \; 2>/dev/null | while read -r line; do echo "  $line"; done
	else
		ls -lh "$dest" 2>/dev/null | while read -r line; do echo "[$(date)] ${label}: $line"; done
	fi
}

pull_remote_file_direct() {
	local remote_path="$1"
	local local_path="$2"
	local label="${3:-拉回}"
	local local_dir remote_bytes

	local_dir="$(dirname "$local_path")"
	mkdir -p "$local_dir"
	remote_bytes=$(ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "stat -c %s $(printf '%q' "$remote_path")" 2>/dev/null || echo 0)
	echo "[$(date)] ${label} 预计拉回大小: $(format_mib "$remote_bytes") MiB"

	scp "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}:${remote_path}" "$local_path"

	echo "[$(date)] ${label} 保存到: $(abs_path "$local_path")"
	print_pull_stats "$local_path" "$label"
}

pull_remote_release_executables() {
	local remote_release_dir="$1"
	local local_release_dir="$2"
	local label="${3:-release}"
	local -a remote_entries=()
	local -a remote_bins=()
	local bin_name bin_size total_bytes

	mkdir -p "$local_release_dir"
	mapfile -t remote_entries < <(
		ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "find '$(printf "%q" "$remote_release_dir")' -maxdepth 1 -type f -executable -printf '%f\t%s\n' | sort"
	)

	if [[ "${#remote_entries[@]}" -eq 0 ]]; then
		echo "[$(date)] no executable artifacts found: ${remote_release_dir}" >&2
		return 1
	fi

	total_bytes=0
	for entry in "${remote_entries[@]}"; do
		[[ -n "$entry" ]] || continue
		bin_name="${entry%%$'\t'*}"
		bin_size="${entry#*$'\t'}"
		remote_bins+=("$bin_name")
		((total_bytes += bin_size))
	done

	echo "[$(date)] ${label} 预计拉回大小: $(format_mib "$total_bytes") MiB"
	echo "[$(date)] 直接同步可执行 bin 到本地 target (${#remote_bins[@]} files)..."
	rsync -az -e "${RSYNC_SSH}" \
		--files-from=<(printf '%s\n' "${remote_bins[@]}") \
		"${REMOTE_USER}@${REMOTE_HOST}:${remote_release_dir}/" \
		"${local_release_dir}/"

	echo "[$(date)] ${label} 保存到: $(abs_path "$local_release_dir")"
	print_pull_stats "$local_release_dir" "$label"
}

ensure_remote_env() {
	if [[ -n "$SKIP_REMOTE_ENV" ]]; then
		return 0
	fi
	echo "[$(date)] 检测远程环境并安装缺失依赖..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "bash -s" <<REMOTE_SCRIPT
set -e
export PATH="\$HOME/.cargo/bin:\$PATH"
if ! command -v cargo &>/dev/null; then
  echo "[remote] 未检测到 cargo，正在安装 rustup..."
  curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain stable
  source "\$HOME/.cargo/env"
fi
if ! rustup target list --installed 2>/dev/null | grep -q '${TARGET}'; then
  echo "[remote] 添加 target ${TARGET}..."
  rustup target add ${TARGET}
fi
if ! command -v aarch64-linux-gnu-gcc &>/dev/null; then
  echo "[remote] 未检测到 aarch64-linux-gnu-gcc，正在安装..."
  if command -v apt-get &>/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq && apt-get install -y -qq gcc-aarch64-linux-gnu libc6-dev-arm64-cross
  elif command -v dnf &>/dev/null; then
    dnf install -y gcc-aarch64-linux-gnu
  elif command -v yum &>/dev/null; then
    yum install -y gcc-aarch64-linux-gnu
  else
    echo "[remote] 无法自动安装 gcc-aarch64-linux-gnu，请手动安装后重试"
    exit 1
  fi
fi
# openssl-sys vendored 构建需要 perl、make
for cmd in perl make; do
  if ! command -v \$cmd &>/dev/null; then
    echo "[remote] 未检测到 \$cmd（openssl vendored 需要），正在安装..."
    if command -v apt-get &>/dev/null; then
      export DEBIAN_FRONTEND=noninteractive
      apt-get update -qq && apt-get install -y -qq \$cmd
    elif command -v dnf &>/dev/null; then
      dnf install -y \$cmd
    elif command -v yum &>/dev/null; then
      yum install -y \$cmd
    fi
  fi
done
echo "[remote] 环境就绪"
REMOTE_SCRIPT
}

skill_to_bin() {
	case "$1" in
	x) echo "x-skill" ;;
	system_basic) echo "system-basic-skill" ;;
	http_basic) echo "http-basic-skill" ;;
	git_basic) echo "git-basic-skill" ;;
	install_module) echo "install-module-skill" ;;
	process_basic) echo "process-basic-skill" ;;
	package_manager) echo "package-manager-skill" ;;
	archive_basic) echo "archive-basic-skill" ;;
	db_basic) echo "db-basic-skill" ;;
	docker_basic) echo "docker-basic-skill" ;;
	fs_search) echo "fs-search-skill" ;;
	rss_fetch) echo "rss-fetch-skill" ;;
	image_vision) echo "image-vision-skill" ;;
	image_generate) echo "image-generate-skill" ;;
	image_edit) echo "image-edit-skill" ;;
	audio_transcribe) echo "audio-transcribe-skill" ;;
	audio_synthesize) echo "audio-synthesize-skill" ;;
	health_check) echo "health-check-skill" ;;
	log_analyze) echo "log-analyze-skill" ;;
	service_control) echo "service-control-skill" ;;
	config_guard) echo "config-guard-skill" ;;
	crypto) echo "crypto-skill" ;;
	chat) echo "chat-skill" ;;
	*) echo "" ;;
	esac
}

usage() {
	local exit_code="${1:-1}"
	echo "用法: $0 [all|skill <技能名>|crate <包名>|dir]"
	echo "  默认仅远程编译，并把 release 中的 bin 直接覆盖回本地 target/release（不拉 debug/非 bin 产物）。"
	echo "  all            - 编译整个 workspace"
	echo "  skill <name>   - 仅编译指定技能，如: skill health_check"
	echo "  crate <name>   - 仅编译指定包，如: crate clawd"
	echo "  dir            - 指定目录上传/拉回，需环境变量："
	echo "      UPLOAD_PATHS  本机相对路径，空格分隔"
	echo "      BUILD_CMD    远程编译命令"
	echo "      PULL_REMOTE  要拉回的远程路径（相对 REMOTE_DIR）"
	echo "      PULL_LOCAL   保存到本机路径"
	echo "  拉完整 release 产物: CROSS_PULL_ALL_ARTIFACTS=1 $0 all"
	exit "$exit_code"
}

MODE="${1:-all}"
PKG="$2"

do_upload() {
	if [[ -n "${UPLOAD_PATHS}" ]]; then
		echo "[$(date)] 上传（仅指定路径）: ${UPLOAD_PATHS}"
		cd "${LOCAL_SOURCE}"
		rsync -az -R -e "${RSYNC_SSH}" \
			$(for p in ${UPLOAD_PATHS}; do echo "./${p}"; done) \
			"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"
	else
		echo "[$(date)] 上传（全部，排除 target/.git）"
		rsync -az --delete -e "${RSYNC_SSH}" \
			--exclude 'target' \
			--exclude '.git' \
			"${LOCAL_SOURCE}/" \
			"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"
	fi
}

case "$MODE" in
all | skill | crate | dir)
	if [[ "${CLEAN_REMOTE_TMP_FIRST}" != "0" ]]; then
		echo "[$(date)] 清理远端临时构建目录: ${REMOTE_DIR}/target"
		ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" \
			"mkdir -p '${REMOTE_DIR}' && rm -rf '${REMOTE_DIR}/target' '${REMOTE_DIR}/tmp' '${REMOTE_DIR}/.cargo-lock' '${REMOTE_DIR}/.rustc_info.json'"
	fi
	do_upload
	ensure_remote_env
	;;
esac

case "$MODE" in
all)
	echo "[$(date)] building full workspace release..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "${REMOTE_CARGO_ENV}${REMOTE_BINDGEN_ENV}cd ${REMOTE_DIR} && cargo build --release --target ${TARGET}"
	RELEASE_DIR="${LOCAL_RELEASE_DIR}"
	mkdir -p "${RELEASE_DIR}"
	if [[ -n "${CROSS_PULL_ALL_ARTIFACTS}" ]]; then
		RSYNC_EXCLUDE=(--exclude='deps/' --exclude='build/' --exclude='incremental/' --exclude='*.rlib' --exclude='*.d')
		REMOTE_RELEASE_BYTES=$(ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "du -sb $(printf '%q' "${REMOTE_DIR}/target/${TARGET}/release") | cut -f1" 2>/dev/null || echo 0)
		echo "[$(date)] release 预计拉回大小: $(format_mib "$REMOTE_RELEASE_BYTES") MiB"
		echo "[$(date)] pulling full release directory (slower)..."
		rsync -az -e "${RSYNC_SSH}" "${RSYNC_EXCLUDE[@]}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/target/${TARGET}/release/" "${RELEASE_DIR}/"
		echo "[$(date)] full release pull completed."
		echo "[$(date)] release saved to: $(abs_path "${RELEASE_DIR}")"
		print_pull_stats "${RELEASE_DIR}" "release"
	else
		pull_remote_release_executables "${REMOTE_DIR}/target/${TARGET}/release" "${RELEASE_DIR}" "release"
	fi
	;;
skill)
	[[ -z "$PKG" ]] && {
		echo "错误: 请指定技能名"
		usage
	}
	BIN_NAME=$(skill_to_bin "$PKG")
	[[ -z "$BIN_NAME" ]] && {
		echo "错误: 未知技能名: $PKG"
		exit 1
	}
	echo "[$(date)] 远程交叉编译技能 ${BIN_NAME}（仅 release）..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "${REMOTE_CARGO_ENV}${REMOTE_BINDGEN_ENV}cd ${REMOTE_DIR} && cargo build -p ${BIN_NAME} --release --target ${TARGET}"
	echo "[$(date)] 正在拉取 release: ${BIN_NAME} ..."
	pull_remote_file_direct \
		"${REMOTE_DIR}/target/${TARGET}/release/${BIN_NAME}" \
		"${LOCAL_RELEASE_DIR}/${BIN_NAME}" \
		"release"
	;;
crate)
	[[ -z "$PKG" ]] && {
		echo "错误: 请指定包名"
		usage
	}
	echo "[$(date)] 远程交叉编译 ${PKG}（仅 release）..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "${REMOTE_CARGO_ENV}${REMOTE_BINDGEN_ENV}cd ${REMOTE_DIR} && cargo build -p ${PKG} --release --target ${TARGET}"
	echo "[$(date)] 正在拉取 release: ${PKG} ..."
	pull_remote_file_direct \
		"${REMOTE_DIR}/target/${TARGET}/release/${PKG}" \
		"${LOCAL_RELEASE_DIR}/${PKG}" \
		"release"
	;;
dir)
	[[ -z "${BUILD_CMD}" ]] && {
		echo "错误: dir 模式必须设置 BUILD_CMD"
		usage
	}
	echo "[$(date)] 远程执行: ${BUILD_CMD}"
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "${REMOTE_CARGO_ENV}${REMOTE_BINDGEN_ENV}cd ${REMOTE_DIR} && ${BUILD_CMD}"
	if [[ -n "${PULL_REMOTE}" ]]; then
		PULL_TO="${PULL_LOCAL:-.}"
		[[ "$PULL_TO" != /* ]] && PULL_TO="${LOCAL_OUTPUT}/${PULL_TO}"
		if ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "test -d ${REMOTE_DIR}/${PULL_REMOTE}" 2>/dev/null; then
			mkdir -p "${PULL_TO}"
			rsync -az -e "${RSYNC_SSH}" \
				"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/${PULL_REMOTE}/" \
				"${PULL_TO}/"
			echo "[$(date)] 保存到: $(abs_path "${PULL_TO}")"
			print_pull_stats "${PULL_TO}" "拉回"
		else
			pull_remote_file_direct \
				"${REMOTE_DIR}/${PULL_REMOTE}" \
				"${PULL_TO}" \
				"拉回"
		fi
	fi
	;;
-h | --help)
	usage 0
	;;
*)
	echo "错误: 未知模式: $MODE"
	usage
	;;
esac
