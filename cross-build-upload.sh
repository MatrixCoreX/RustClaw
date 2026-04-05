#!/usr/bin/env bash
# 上传 RustClaw 到高配机，远程交叉编译 aarch64，结束后取回
# 用法: ./cross-build-upload.sh [all|skill <name>|crate <name>|dir]
#  dir 模式用环境变量指定上传/拉回：UPLOAD_PATHS BUILD_CMD PULL_REMOTE PULL_LOCAL
# 依赖: 远程可为 Linux/macOS，脚本会自动检测并安装对应交叉编译依赖

set -e
SKIP_REMOTE_ENV="${SKIP_REMOTE_ENV:-}"
CROSS_PULL_ALL_ARTIFACTS="${CROSS_PULL_ALL_ARTIFACTS:-}"
CLEAN_REMOTE_TMP_FIRST="${CLEAN_REMOTE_TMP_FIRST:-0}"
SHOW_RSYNC_PROGRESS="${SHOW_RSYNC_PROGRESS:-1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
REMOTE_USER="${REMOTE_USER:-testuser}"
REMOTE_HOST="${REMOTE_HOST:-192.168.31.162}"
if [[ -z "${REMOTE_SSH_KEY}" ]]; then
	if [[ -f "${HOME}/.ssh/id_ed25519" ]]; then
		REMOTE_SSH_KEY="${HOME}/.ssh/id_ed25519"
	else
		REMOTE_SSH_KEY="${HOME}/.ssh/id_rsa"
	fi
fi
REMOTE_DIR="${REMOTE_DIR:-/tmp/rustclaw-cross-build}"
LOCAL_SOURCE="${SCRIPT_DIR}"
LOCAL_OUTPUT="${SCRIPT_DIR}"
TARGET="aarch64-unknown-linux-gnu"
LOCAL_RELEASE_DIR="${LOCAL_OUTPUT}/target/release"

abs_path() { echo "$(cd "$(dirname "$1")" 2>/dev/null && pwd)/$(basename "$1")" || echo "$1"; }
format_mib() { awk -v bytes="${1:-0}" 'BEGIN { printf "%.2f", bytes / 1048576 }'; }

SSH_OPTS=(-i "${REMOTE_SSH_KEY}")
RSYNC_SSH="ssh -i ${REMOTE_SSH_KEY}"
RSYNC_PROGRESS_OPTS=()
if [[ "${SHOW_RSYNC_PROGRESS}" != "0" ]]; then
	RSYNC_PROGRESS_OPTS=(--info=progress2 --human-readable)
fi
REMOTE_SHELL_INIT='source ~/.cargo/env 2>/dev/null; export PATH="$HOME/.cargo/bin:$PATH"; if [[ "$(uname -s)" == "Darwin" ]]; then if [[ -x /opt/homebrew/bin/brew ]]; then eval "$(/opt/homebrew/bin/brew shellenv)"; elif [[ -x /usr/local/bin/brew ]]; then eval "$(/usr/local/bin/brew shellenv)"; fi; if command -v brew >/dev/null 2>&1; then TOOLCHAIN_PREFIX="$(brew --prefix aarch64-unknown-linux-gnu 2>/dev/null || true)"; if [[ -n "$TOOLCHAIN_PREFIX" && -d "$TOOLCHAIN_PREFIX/bin" ]]; then export PATH="$TOOLCHAIN_PREFIX/bin:$PATH"; fi; fi; fi; '
REMOTE_CARGO_ENV="${REMOTE_SHELL_INIT}"
# 统一探测交叉工具链名字，并导出给 cargo/cc-rs 使用
REMOTE_TOOLCHAIN_ENV='CROSS_GCC_BIN=""; for candidate in aarch64-linux-gnu-gcc aarch64-unknown-linux-gnu-gcc; do if command -v "$candidate" >/dev/null 2>&1; then CROSS_GCC_BIN="$(command -v "$candidate")"; break; fi; done; if [[ -n "$CROSS_GCC_BIN" ]]; then export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$CROSS_GCC_BIN"; export CC_aarch64_unknown_linux_gnu="$CROSS_GCC_BIN"; CROSS_BIN_DIR="$(dirname "$CROSS_GCC_BIN")"; CROSS_BIN_PREFIX="$(basename "$CROSS_GCC_BIN")"; CROSS_BIN_PREFIX="${CROSS_BIN_PREFIX%gcc}"; if [[ -x "${CROSS_BIN_DIR}/${CROSS_BIN_PREFIX}g++" ]]; then export CXX_aarch64_unknown_linux_gnu="${CROSS_BIN_DIR}/${CROSS_BIN_PREFIX}g++"; fi; if [[ -x "${CROSS_BIN_DIR}/${CROSS_BIN_PREFIX}ar" ]]; then export AR_aarch64_unknown_linux_gnu="${CROSS_BIN_DIR}/${CROSS_BIN_PREFIX}ar"; fi; fi; '
# bindgen 在 aarch64 交叉编译时需要显式看到目标头文件，否则 silk-rs 会报 float.h not found
REMOTE_BINDGEN_ENV='if [[ -z "${CROSS_GCC_BIN:-}" ]]; then for candidate in aarch64-linux-gnu-gcc aarch64-unknown-linux-gnu-gcc; do if command -v "$candidate" >/dev/null 2>&1; then CROSS_GCC_BIN="$(command -v "$candidate")"; break; fi; done; fi; if [[ -n "${CROSS_GCC_BIN:-}" && -x "$CROSS_GCC_BIN" ]]; then GCC_INCLUDE_DIR="$("$CROSS_GCC_BIN" -print-file-name=include 2>/dev/null)"; GCC_SYSROOT="$("$CROSS_GCC_BIN" -print-sysroot 2>/dev/null)"; TARGET_INCLUDE_DIR=""; for candidate in "$GCC_SYSROOT/usr/include" "$GCC_SYSROOT/include" "/usr/aarch64-linux-gnu/include"; do if [[ -n "$candidate" && -d "$candidate" ]]; then TARGET_INCLUDE_DIR="$candidate"; break; fi; done; if [[ -n "$GCC_INCLUDE_DIR" && -d "$GCC_INCLUDE_DIR" ]]; then EXTRA_ARGS="--target=aarch64-linux-gnu -I$GCC_INCLUDE_DIR"; if [[ -n "$TARGET_INCLUDE_DIR" ]]; then EXTRA_ARGS="$EXTRA_ARGS -I$TARGET_INCLUDE_DIR"; fi; export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_gnu="$EXTRA_ARGS"; fi; fi; '

remote_exec() {
	local command="$1"
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" \
		"bash -lc $(printf '%q' "${REMOTE_SHELL_INIT}${command}")"
}

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
	remote_bytes=$(remote_exec "if [[ \"\$(uname -s)\" == \"Darwin\" ]]; then stat -f %z $(printf '%q' "$remote_path"); else stat -c %s $(printf '%q' "$remote_path"); fi" 2>/dev/null || echo 0)
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
	local bin_name bin_size total_bytes remote_entries_raw

	mkdir -p "$local_release_dir"
	remote_entries_raw="$(
		remote_exec "REMOTE_RELEASE_DIR=$(printf '%q' "$remote_release_dir"); shopt -s nullglob; for f in \"\$REMOTE_RELEASE_DIR\"/*; do [[ -f \"\$f\" && -x \"\$f\" ]] || continue; if [[ \"\$(uname -s)\" == \"Darwin\" ]]; then size=\$(stat -f %z \"\$f\"); else size=\$(stat -c %s \"\$f\"); fi; printf '%s\t%s\n' \"\$(basename \"\$f\")\" \"\$size\"; done | sort"
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

	echo "[$(date)] ${label} 预计拉回大小: $(format_mib "$total_bytes") MiB"
	echo "[$(date)] 直接同步可执行 bin 到本地 target (${#remote_bins[@]} files)..."
	rsync -az -e "${RSYNC_SSH}" \
		"${RSYNC_PROGRESS_OPTS[@]}" \
		--files-from=<(printf '%s\n' "${remote_bins[@]}") \
		"${REMOTE_USER}@${REMOTE_HOST}:${remote_release_dir}/" \
		"${local_release_dir}/"

	echo "[$(date)] ${label} 保存到: $(abs_path "$local_release_dir")"
	print_pull_stats "$local_release_dir" "$label"
}

ensure_remote_dir() {
	echo "[$(date)] 确保远端构建目录存在: ${REMOTE_DIR}"
	remote_exec "mkdir -p $(printf '%q' "${REMOTE_DIR}")"
}

ensure_remote_env() {
	if [[ -n "$SKIP_REMOTE_ENV" ]]; then
		return 0
	fi
	echo "[$(date)] 检测远程环境并安装缺失依赖..."
	ssh "${SSH_OPTS[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "bash -s" <<REMOTE_SCRIPT
set -e
export PATH="\$HOME/.cargo/bin:\$PATH"
REMOTE_OS="\$(uname -s)"
if [[ "\$REMOTE_OS" == "Darwin" ]]; then
  if ! command -v brew &>/dev/null; then
    if [[ -x /opt/homebrew/bin/brew ]]; then
      eval "\$(/opt/homebrew/bin/brew shellenv)"
    elif [[ -x /usr/local/bin/brew ]]; then
      eval "\$(/usr/local/bin/brew shellenv)"
    fi
  fi
fi
echo "[remote] 检测到系统: \$REMOTE_OS"
brew_install_with_lock_retry() {
  local formula="\$1"
  local max_attempts="\${2:-30}"
  local sleep_seconds="\${3:-10}"
  local attempt=1
  local output=""
  local wait_total=0
  local log_file=""
  while (( attempt <= max_attempts )); do
    if brew list --versions "\$formula" >/dev/null 2>&1; then
      echo "[remote] brew 包已存在: \$formula"
      return 0
    fi
    echo "[remote] brew install \$formula (attempt \$attempt/\$max_attempts)"
    echo "[remote] 以下为 brew 实时输出："
    log_file="\$(mktemp)"
    set +e
    brew install "\$formula" 2>&1 | tee "\$log_file"
    status=\${PIPESTATUS[0]}
    set -e
    output="\$(cat "\$log_file")"
    rm -f "\$log_file"
    log_file=""
    if [[ \$status -eq 0 ]]; then
      return 0
    fi
    if grep -qi 'already locked' <<<"\$output"; then
      wait_total=\$((wait_total + sleep_seconds))
      echo "[remote] Homebrew 正在被其他进程占用（已等待 \${wait_total}s，重试 \$attempt/\$max_attempts），\${sleep_seconds}s 后继续..."
      sleep "\$sleep_seconds"
      ((attempt += 1))
      continue
    fi
    printf '%s\n' "\$output" >&2
    return \$status
  done
  echo "[remote] brew install \$formula 超时：长时间被其他 Homebrew 进程占用，疑似卡死。" >&2
  echo "[remote] 请到远端检查 brew 进程是否卡住，例如：" >&2
  echo "[remote]   ps aux | grep '[b]rew'" >&2
  echo "[remote] 如果确认是僵死/卡死进程，先结束它，再重新运行脚本。" >&2
  [[ -n "\$log_file" && -f "\$log_file" ]] && rm -f "\$log_file"
  return 1
}
if ! command -v cargo &>/dev/null; then
  echo "[remote] 未检测到 cargo，正在安装 rustup..."
  curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain stable
  source "\$HOME/.cargo/env"
fi
if ! rustup target list --installed 2>/dev/null | grep -q '${TARGET}'; then
  echo "[remote] 添加 target ${TARGET}..."
  rustup target add ${TARGET}
fi
if ! command -v aarch64-linux-gnu-gcc &>/dev/null && ! command -v aarch64-unknown-linux-gnu-gcc &>/dev/null; then
  echo "[remote] 未检测到 aarch64-linux-gnu-gcc，正在安装..."
  if [[ "\$REMOTE_OS" == "Darwin" ]]; then
    if ! command -v brew &>/dev/null; then
      echo "[remote] macOS 未检测到 Homebrew，请先安装 brew 后重试"
      exit 1
    fi
    brew tap messense/macos-cross-toolchains
    brew_install_with_lock_retry aarch64-unknown-linux-gnu
    TOOLCHAIN_PREFIX="\$(brew --prefix aarch64-unknown-linux-gnu 2>/dev/null || true)"
    if [[ -n "\$TOOLCHAIN_PREFIX" && -d "\$TOOLCHAIN_PREFIX/bin" ]]; then
      export PATH="\$TOOLCHAIN_PREFIX/bin:\$PATH"
    fi
  elif command -v apt-get &>/dev/null; then
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
if ! command -v aarch64-linux-gnu-gcc &>/dev/null && ! command -v aarch64-unknown-linux-gnu-gcc &>/dev/null; then
  echo "[remote] 依赖安装完成后仍未找到 aarch64-linux-gnu-gcc，请检查交叉工具链 PATH"
  exit 1
fi
# openssl-sys vendored 构建需要 perl、make
for cmd in perl make; do
  if ! command -v \$cmd &>/dev/null; then
    echo "[remote] 未检测到 \$cmd（openssl vendored 需要），正在安装..."
    if [[ "\$REMOTE_OS" == "Darwin" ]]; then
      if [[ "\$cmd" == "perl" ]]; then
        brew_install_with_lock_retry perl
      else
        echo "[remote] macOS 未找到 \$cmd，请先安装 Xcode Command Line Tools 或手动补齐后重试"
        exit 1
      fi
    elif command -v apt-get &>/dev/null; then
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

crate_to_bins() {
	local package_name="$1"
	python3 - "$SCRIPT_DIR" "$package_name" <<'PY'
import pathlib
import sys
import tomllib

root = pathlib.Path(sys.argv[1])
package_name = sys.argv[2]

workspace_manifest = root / "Cargo.toml"
workspace_data = tomllib.loads(workspace_manifest.read_text())
members = workspace_data.get("workspace", {}).get("members", [])

manifests = [workspace_manifest]
seen = {workspace_manifest.resolve()}

for member in members:
    for path in sorted(root.glob(member)):
        manifest = path / "Cargo.toml"
        if manifest.is_file():
            real = manifest.resolve()
            if real not in seen:
                manifests.append(manifest)
                seen.add(real)

for manifest in manifests:
    data = tomllib.loads(manifest.read_text())
    package = data.get("package", {})
    if package.get("name") != package_name:
        continue

    bins = []
    autobins = package.get("autobins", True)
    for target in data.get("bin", []):
        name = (target or {}).get("name")
        if isinstance(name, str) and name.strip():
            bins.append(name.strip())

    if autobins:
        src_main = manifest.parent / "src/main.rs"
        if src_main.is_file():
            default_name = package.get("name", "").strip()
            if default_name:
                bins.append(default_name)

        src_bin = manifest.parent / "src/bin"
        if src_bin.is_dir():
            for entry in sorted(src_bin.iterdir()):
                if entry.is_file() and entry.suffix == ".rs":
                    bins.append(entry.stem)
                elif entry.is_dir() and (entry / "main.rs").is_file():
                    bins.append(entry.name)

    for name in sorted(set(bins)):
        print(name)
    sys.exit(0)

print(f"错误: 未找到 crate/package: {package_name}", file=sys.stderr)
sys.exit(1)
PY
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
	ensure_remote_dir
	if [[ -n "${UPLOAD_PATHS}" ]]; then
		echo "[$(date)] 上传（仅指定路径）: ${UPLOAD_PATHS}"
		cd "${LOCAL_SOURCE}"
		rsync -az -R -e "${RSYNC_SSH}" \
			"${RSYNC_PROGRESS_OPTS[@]}" \
			$(for p in ${UPLOAD_PATHS}; do echo "./${p}"; done) \
			"${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"
	else
		echo "[$(date)] 上传（全部，排除 target/.git/UI/services/根 node_modules）"
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
		echo "[$(date)] 清理远端临时构建目录: ${REMOTE_DIR}/target"
		remote_exec "mkdir -p $(printf '%q' "${REMOTE_DIR}") && rm -rf $(printf '%q' "${REMOTE_DIR}/target") $(printf '%q' "${REMOTE_DIR}/tmp") $(printf '%q' "${REMOTE_DIR}/.cargo-lock") $(printf '%q' "${REMOTE_DIR}/.rustc_info.json")"
	fi
	do_upload
	ensure_remote_env
	;;
esac

case "$MODE" in
all)
	echo "[$(date)] building full workspace release..."
	remote_exec "${REMOTE_CARGO_ENV}${REMOTE_TOOLCHAIN_ENV}${REMOTE_BINDGEN_ENV}cd $(printf '%q' "${REMOTE_DIR}") && cargo build --release --target ${TARGET}"
	RELEASE_DIR="${LOCAL_RELEASE_DIR}"
	mkdir -p "${RELEASE_DIR}"
	if [[ -n "${CROSS_PULL_ALL_ARTIFACTS}" ]]; then
		RSYNC_EXCLUDE=(--exclude='deps/' --exclude='build/' --exclude='incremental/' --exclude='*.rlib' --exclude='*.d')
		REMOTE_RELEASE_BYTES=$(remote_exec "if du -sb $(printf '%q' "${REMOTE_DIR}/target/${TARGET}/release") >/dev/null 2>&1; then du -sb $(printf '%q' "${REMOTE_DIR}/target/${TARGET}/release") | cut -f1; else du -sk $(printf '%q' "${REMOTE_DIR}/target/${TARGET}/release") | awk '{print \$1 * 1024}'; fi" 2>/dev/null || echo 0)
		echo "[$(date)] release 预计拉回大小: $(format_mib "$REMOTE_RELEASE_BYTES") MiB"
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
		echo "错误: 请指定技能名"
		usage
	}
	BIN_NAME=$(skill_to_bin "$PKG")
	[[ -z "$BIN_NAME" ]] && {
		echo "错误: 未知技能名: $PKG"
		exit 1
	}
	echo "[$(date)] 远程交叉编译技能 ${BIN_NAME}（仅 release）..."
	remote_exec "${REMOTE_CARGO_ENV}${REMOTE_TOOLCHAIN_ENV}${REMOTE_BINDGEN_ENV}cd $(printf '%q' "${REMOTE_DIR}") && cargo build -p ${BIN_NAME} --release --target ${TARGET}"
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
	BIN_NAMES_RAW="$(crate_to_bins "$PKG")"
	array_from_string_lines BIN_NAMES "$BIN_NAMES_RAW"
	[[ "${#BIN_NAMES[@]}" -eq 0 ]] && {
		echo "错误: crate ${PKG} 没有可拉回的 bin 目标"
		exit 1
	}
	echo "[$(date)] 远程交叉编译 ${PKG}（仅 release）..."
	remote_exec "${REMOTE_CARGO_ENV}${REMOTE_TOOLCHAIN_ENV}${REMOTE_BINDGEN_ENV}cd $(printf '%q' "${REMOTE_DIR}") && cargo build -p ${PKG} --release --target ${TARGET}"
	echo "[$(date)] 正在拉取 crate ${PKG} 的 release bin: ${BIN_NAMES[*]} ..."
	for bin_name in "${BIN_NAMES[@]}"; do
		pull_remote_file_direct \
			"${REMOTE_DIR}/target/${TARGET}/release/${bin_name}" \
			"${LOCAL_RELEASE_DIR}/${bin_name}" \
			"release"
	done
	;;
dir)
	[[ -z "${BUILD_CMD}" ]] && {
		echo "错误: dir 模式必须设置 BUILD_CMD"
		usage
	}
	echo "[$(date)] 远程执行: ${BUILD_CMD}"
	remote_exec "${REMOTE_CARGO_ENV}${REMOTE_TOOLCHAIN_ENV}${REMOTE_BINDGEN_ENV}cd $(printf '%q' "${REMOTE_DIR}") && ${BUILD_CMD}"
	if [[ -n "${PULL_REMOTE}" ]]; then
		PULL_TO="${PULL_LOCAL:-.}"
		[[ "$PULL_TO" != /* ]] && PULL_TO="${LOCAL_OUTPUT}/${PULL_TO}"
		if remote_exec "test -d $(printf '%q' "${REMOTE_DIR}/${PULL_REMOTE}")" 2>/dev/null; then
				mkdir -p "${PULL_TO}"
				rsync -az -e "${RSYNC_SSH}" \
					"${RSYNC_PROGRESS_OPTS[@]}" \
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
