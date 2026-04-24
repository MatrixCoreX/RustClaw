#!/usr/bin/env bash
# 上传 RustClaw 到高配机，远程交叉编译 aarch64，结束后取回
# 用法: ./cross-build-upload.sh [all|skill <name>|crate <name>|dir]
#  dir 模式用环境变量指定上传/拉回：UPLOAD_PATHS BUILD_CMD PULL_REMOTE PULL_LOCAL
# 依赖: 远程为 Linux，脚本会自动检测并安装 rustup/target/gcc-aarch64-linux-gnu

set -e
SKIP_REMOTE_ENV="${SKIP_REMOTE_ENV:-}"
CROSS_PULL_ALL_ARTIFACTS="${CROSS_PULL_ALL_ARTIFACTS:-}"
CLEAN_REMOTE_TMP_FIRST="${CLEAN_REMOTE_TMP_FIRST:-0}"
SHOW_RSYNC_PROGRESS="${SHOW_RSYNC_PROGRESS:-1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
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
TARGET="${TARGET:-${RUSTCLAW_CROSS_TARGET:-aarch64-unknown-linux-gnu}}"
LOCAL_RELEASE_DIR="$(target_release_dir "${LOCAL_OUTPUT}" "${TARGET}")"
HOST_OS="$(detect_host_os || printf '%s' "unknown")"
HOST_ARCH="$(detect_host_arch || printf '%s' "unknown")"

abs_path() { echo "$(cd "$(dirname "$1")" 2>/dev/null && pwd)/$(basename "$1")" || echo "$1"; }
format_mib() { awk -v bytes="${1:-0}" 'BEGIN { printf "%.2f", bytes / 1048576 }'; }

SSH_OPTS=(-i "${REMOTE_SSH_KEY}")
RSYNC_SSH="ssh -i ${REMOTE_SSH_KEY}"
RSYNC_PROGRESS_OPTS=()
if [[ "${SHOW_RSYNC_PROGRESS}" != "0" ]]; then
	RSYNC_PROGRESS_OPTS=(--info=progress2 --human-readable)
fi
REMOTE_CARGO_ENV='source ~/.cargo/env 2>/dev/null; export PATH="$HOME/.cargo/bin:$PATH"; '
# bindgen 在 aarch64 交叉编译时需要显式看到目标头文件，否则 silk-rs 会报 float.h not found
REMOTE_BINDGEN_ENV='GCC_INCLUDE_DIR="$(aarch64-linux-gnu-gcc -print-file-name=include 2>/dev/null)"; TARGET_INCLUDE_DIR="/usr/aarch64-linux-gnu/include"; if [[ -n "$GCC_INCLUDE_DIR" && -d "$TARGET_INCLUDE_DIR" ]]; then export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_gnu="--target=aarch64-linux-gnu -I$GCC_INCLUDE_DIR -I$TARGET_INCLUDE_DIR"; fi; '

# 统计并打印拉回产物大小（目录或单个文件）
print_pull_stats() {
	local dest="$1"
	local label="${2:-pullback}"
	if [[ ! -e "$dest" ]]; then
		echo "[$(date)] ${label}: path does not exist: $dest"
		return
	fi
	if [[ -d "$dest" ]]; then
		local total
		total=$(du -sh "$dest" 2>/dev/null | cut -f1)
		echo "[$(date)] ${label} total size: ${total}"
		echo "[$(date)] file list (excluding deps/build/*.rlib/*.d):"
		find "$dest" -maxdepth 1 -type f \( ! -name '*.rlib' ! -name '*.d' \) -exec ls -lh {} \; 2>/dev/null | while read -r line; do echo "  $line"; done
	else
		ls -lh "$dest" 2>/dev/null | while read -r line; do echo "[$(date)] ${label}: $line"; done
	fi
}

pull_remote_file_direct() {
	local remote_path="$1"
	local local_path="$2"
	local label="${3:-pullback}"
	local local_dir remote_bytes tmp_local_path

	local_dir="$(dirname "$local_path")"
	mkdir -p "$local_dir"
	remote_bytes=$(ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "stat -c %s $(printf '%q' "$remote_path")" 2>/dev/null || echo 0)
	echo "[$(date)] ${label} estimated download size: $(format_mib "$remote_bytes") MiB"

	tmp_local_path="${local_path}.tmp.$$"
	rm -f "$tmp_local_path"
	scp "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}:${remote_path}" "$tmp_local_path"
	mv -f "$tmp_local_path" "$local_path"

	echo "[$(date)] ${label} saved to: $(abs_path "$local_path")"
	print_pull_stats "$local_path" "$label"
}

pull_remote_release_executables() {
	local remote_release_dir="$1"
	local local_release_dir="$2"
	local label="${3:-release}"
	local -a remote_entries=()
	local -a remote_bins=()
	local bin_name bin_size total_bytes stage_dir remote_entries_raw

	mkdir -p "$local_release_dir"
	remote_entries_raw="$(
		ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "find '$(printf "%q" "$remote_release_dir")' -maxdepth 1 -type f -executable -printf '%f\t%s\n' | sort"
	)"
	array_from_string_lines remote_entries "$remote_entries_raw"

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

	echo "[$(date)] ${label} estimated download size: $(format_mib "$total_bytes") MiB"
	echo "[$(date)] syncing executable bins directly to local target (${#remote_bins[@]} files)..."
	stage_dir="${local_release_dir}/.pull-stage.$$"
	rm -rf "$stage_dir"
	mkdir -p "$stage_dir"
	rsync -az -e "${RSYNC_SSH}" \
		"${RSYNC_PROGRESS_OPTS[@]}" \
		--files-from=<(printf '%s\n' "${remote_bins[@]}") \
		"${REMOTE_USER}@${REMOTE_HOST}:${remote_release_dir}/" \
		"${stage_dir}/"
	for bin_name in "${remote_bins[@]}"; do
		mv -f "${stage_dir}/${bin_name}" "${local_release_dir}/${bin_name}"
	done
	rmdir "$stage_dir" 2>/dev/null || true

	echo "[$(date)] ${label} saved to: $(abs_path "$local_release_dir")"
	print_pull_stats "$local_release_dir" "$label"
}

ensure_remote_env() {
	if [[ -n "$SKIP_REMOTE_ENV" ]]; then
		return 0
	fi
	echo "[$(date)] checking remote environment and installing missing dependencies..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "bash -s" <<REMOTE_SCRIPT
set -e
export PATH="\$HOME/.cargo/bin:\$PATH"
if ! command -v cargo &>/dev/null; then
  echo "[remote] cargo not found, installing rustup..."
  curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain stable
  source "\$HOME/.cargo/env"
fi
if ! rustup target list --installed 2>/dev/null | grep -q '${TARGET}'; then
  echo "[remote] adding target ${TARGET}..."
  rustup target add ${TARGET}
fi
if ! command -v aarch64-linux-gnu-gcc &>/dev/null; then
  echo "[remote] aarch64-linux-gnu-gcc not found, installing..."
  if command -v apt-get &>/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq && apt-get install -y -qq gcc-aarch64-linux-gnu libc6-dev-arm64-cross
  elif command -v dnf &>/dev/null; then
    dnf install -y gcc-aarch64-linux-gnu
  elif command -v yum &>/dev/null; then
    yum install -y gcc-aarch64-linux-gnu
  else
    echo "[remote] cannot auto-install gcc-aarch64-linux-gnu; install it manually and retry"
    exit 1
  fi
fi
# openssl-sys vendored 构建需要 perl、make
for cmd in perl make; do
  if ! command -v \$cmd &>/dev/null; then
    echo "[remote] \$cmd not found (required by vendored openssl); installing..."
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
echo "[remote] environment ready"
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
	*) echo "" ;;
	esac
}

usage() {
	local exit_code="${1:-1}"
	# zh: 打印云端远程交叉编译脚本的英文用法；中文说明保留在维护注释中。
	echo "Usage: $0 [all|skill <skill-name>|crate <package-name>|dir]"
	echo "  By default, builds remotely and copies release bins back to local target/release only."
	echo "  Host platform: ${HOST_OS}/${HOST_ARCH}; cross target: ${TARGET}"
	echo "  all            - build the whole workspace"
	echo "  skill <name>   - build one skill, e.g. skill health_check"
	echo "  crate <name>   - build one package, e.g. crate clawd"
	echo "  dir            - upload/pull selected paths; requires environment variables:"
	echo "      UPLOAD_PATHS  local relative paths, space-separated"
	echo "      BUILD_CMD     remote build command"
	echo "      PULL_REMOTE   remote path to pull back, relative to REMOTE_DIR"
	echo "      PULL_LOCAL    local destination path"
	echo "  Pull the full release artifacts: CROSS_PULL_ALL_ARTIFACTS=1 $0 all"
	exit "$exit_code"
}

MODE="${1:-all}"
PKG="$2"

do_upload() {
	if [[ -n "${UPLOAD_PATHS}" ]]; then
		echo "[$(date)] uploading selected paths only: ${UPLOAD_PATHS}"
		cd "${LOCAL_SOURCE}"
		rsync -az -R -e "${RSYNC_SSH}" \
			"${RSYNC_PROGRESS_OPTS[@]}" \
			$(for p in ${UPLOAD_PATHS}; do echo "./${p}"; done) \
			"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"
	else
		echo "[$(date)] uploading full repo (excluding target/.git/UI/services/root node_modules)"
		rsync -az --delete -e "${RSYNC_SSH}" \
			"${RSYNC_PROGRESS_OPTS[@]}" \
			--exclude 'target' \
			--exclude '.git' \
			--exclude '/UI' \
			--exclude '/services' \
			--exclude '/node_modules' \
			"${LOCAL_SOURCE}/" \
			"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"
	fi
}

case "$MODE" in
all | skill | crate | dir)
	if [[ "${CLEAN_REMOTE_TMP_FIRST}" != "0" ]]; then
		echo "[$(date)] cleaning remote temporary build directory: ${REMOTE_DIR}/target"
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
		echo "[$(date)] release estimated download size: $(format_mib "$REMOTE_RELEASE_BYTES") MiB"
			echo "[$(date)] pulling full release directory (slower)..."
			rsync -az -e "${RSYNC_SSH}" "${RSYNC_PROGRESS_OPTS[@]}" "${RSYNC_EXCLUDE[@]}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/target/${TARGET}/release/" "${RELEASE_DIR}/"
			echo "[$(date)] full release pull completed."
			echo "[$(date)] release saved to: $(abs_path "${RELEASE_DIR}")"
			print_pull_stats "${RELEASE_DIR}" "release"
	else
		pull_remote_release_executables "${REMOTE_DIR}/target/${TARGET}/release" "${RELEASE_DIR}" "release"
	fi
	;;
skill)
	[[ -z "$PKG" ]] && {
		echo "Error: skill name is required"
		usage
	}
	BIN_NAME=$(skill_to_bin "$PKG")
	[[ -z "$BIN_NAME" ]] && {
		echo "Error: unknown skill name: $PKG"
		exit 1
	}
	echo "[$(date)] remote cross-building skill ${BIN_NAME} (release only)..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "${REMOTE_CARGO_ENV}${REMOTE_BINDGEN_ENV}cd ${REMOTE_DIR} && cargo build -p ${BIN_NAME} --release --target ${TARGET}"
	echo "[$(date)] pulling release: ${BIN_NAME} ..."
	pull_remote_file_direct \
		"${REMOTE_DIR}/target/${TARGET}/release/${BIN_NAME}" \
		"${LOCAL_RELEASE_DIR}/${BIN_NAME}" \
		"release"
	;;
crate)
	[[ -z "$PKG" ]] && {
		echo "Error: package name is required"
		usage
	}
	echo "[$(date)] remote cross-building ${PKG} (release only)..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "${REMOTE_CARGO_ENV}${REMOTE_BINDGEN_ENV}cd ${REMOTE_DIR} && cargo build -p ${PKG} --release --target ${TARGET}"
	echo "[$(date)] pulling release: ${PKG} ..."
	pull_remote_file_direct \
		"${REMOTE_DIR}/target/${TARGET}/release/${PKG}" \
		"${LOCAL_RELEASE_DIR}/${PKG}" \
		"release"
	;;
dir)
	[[ -z "${BUILD_CMD}" ]] && {
		echo "Error: dir mode requires BUILD_CMD"
		usage
	}
	echo "[$(date)] remote command: ${BUILD_CMD}"
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "${REMOTE_CARGO_ENV}${REMOTE_BINDGEN_ENV}cd ${REMOTE_DIR} && ${BUILD_CMD}"
	if [[ -n "${PULL_REMOTE}" ]]; then
		PULL_TO="${PULL_LOCAL:-.}"
		[[ "$PULL_TO" != /* ]] && PULL_TO="${LOCAL_OUTPUT}/${PULL_TO}"
		if ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "test -d ${REMOTE_DIR}/${PULL_REMOTE}" 2>/dev/null; then
				mkdir -p "${PULL_TO}"
				rsync -az -e "${RSYNC_SSH}" \
					"${RSYNC_PROGRESS_OPTS[@]}" \
					"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/${PULL_REMOTE}/" \
					"${PULL_TO}/"
				echo "[$(date)] saved to: $(abs_path "${PULL_TO}")"
			print_pull_stats "${PULL_TO}" "pullback"
		else
			pull_remote_file_direct \
				"${REMOTE_DIR}/${PULL_REMOTE}" \
				"${PULL_TO}" \
				"pullback"
		fi
	fi
	;;
-h | --help)
	usage 0
	;;
*)
	echo "Error: unknown mode: $MODE"
	usage
	;;
esac
