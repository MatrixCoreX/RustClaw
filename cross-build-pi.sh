#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
cd "${SCRIPT_DIR}"

TARGET="${RUSTCLAW_PI_TARGET:-aarch64-unknown-linux-gnu}"
BUILD_SCOPE="workspace"
PACKAGE_NAME=""
BIN_NAME=""
DO_CLEAN=0
SKIP_UI=1
INSTALL_DEPS="${INSTALL_DEPS:-1}"
SYNC_RELEASE_BIN=0
CARGO_EXTRA_ARGS=()

log() {
	echo "[$(date '+%F %T')] $*" >&2
}

die() {
	echo "[$(date '+%F %T')] error: $*" >&2
	exit 1
}

usage() {
	cat <<'EOF'
Usage: ./cross-build-pi.sh [options] [-- cargo-args...]

Cross-build RustClaw locally for Raspberry Pi Linux.

Targets:
  --target pi64                         aarch64-unknown-linux-gnu (default)
  --target pi32                         armv7-unknown-linux-gnueabihf
  --target <rust-target-triple>          explicit Rust target triple

Build selection:
  --workspace                            build the whole workspace (default)
  --package <name>                       build one Cargo package
  --bin <name>                           build one binary target

Options:
  --clean                                run cargo clean first
  --with-ui                              let build-all.sh build/deploy UI assets
  --no-ui                                skip UI build when building workspace (default)
  --skip-deps                            do not install cross-build dependencies
  --sync-release-bin                     copy target/<triple>/release executables to release-bin/
  -h, --help                             show this help

Environment:
  RUSTCLAW_PI_TARGET                     default target override
  INSTALL_DEPS=0                         same as --skip-deps

Examples:
  ./cross-build-pi.sh
  ./cross-build-pi.sh --target pi32
  ./cross-build-pi.sh --package clawd
  ./cross-build-pi.sh --bin clawd -- --locked
EOF
}

normalize_target() {
	case "${1:-}" in
	""|pi64|raspi64|rpi64)
		printf '%s\n' "aarch64-unknown-linux-gnu"
		;;
	pi32|raspi32|rpi32)
		printf '%s\n' "armv7-unknown-linux-gnueabihf"
		;;
	*)
		printf '%s\n' "$1"
		;;
	esac
}

target_env_key() {
	printf '%s\n' "$1" | tr '[:lower:]-' '[:upper:]_'
}

target_cc_key() {
	printf '%s\n' "$1" | tr '-' '_'
}

target_clang_triple() {
	case "$1" in
	aarch64-unknown-linux-gnu)
		printf '%s\n' "aarch64-linux-gnu"
		;;
	armv7-unknown-linux-gnueabihf)
		printf '%s\n' "armv7-unknown-linux-gnueabihf"
		;;
	*)
		printf '%s\n' "$1"
		;;
	esac
}

target_apt_packages() {
	case "$1" in
	aarch64-unknown-linux-gnu)
		printf '%s\n' "gcc-aarch64-linux-gnu libc6-dev-arm64-cross pkg-config make perl clang libclang-dev protobuf-compiler"
		;;
	armv7-unknown-linux-gnueabihf)
		printf '%s\n' "gcc-arm-linux-gnueabihf libc6-dev-armhf-cross pkg-config make perl clang libclang-dev protobuf-compiler"
		;;
	*)
		return 1
		;;
	esac
}

target_brew_formula() {
	case "$1" in
	aarch64-unknown-linux-gnu)
		printf '%s\n' "aarch64-unknown-linux-gnu"
		;;
	armv7-unknown-linux-gnueabihf)
		printf '%s\n' "armv7-unknown-linux-gnueabihf"
		;;
	*)
		return 1
		;;
	esac
}

linker_candidates() {
	case "$1" in
	aarch64-unknown-linux-gnu)
		printf '%s\n' "aarch64-linux-gnu-gcc"
		printf '%s\n' "aarch64-unknown-linux-gnu-gcc"
		;;
	armv7-unknown-linux-gnueabihf)
		printf '%s\n' "arm-linux-gnueabihf-gcc"
		printf '%s\n' "armv7-unknown-linux-gnueabihf-gcc"
		;;
	*)
		printf '%s\n' "${1}-gcc"
		;;
	esac
}

ensure_cargo() {
	if [[ -f "${HOME}/.cargo/env" ]]; then
		# shellcheck source=/dev/null
		source "${HOME}/.cargo/env"
	fi
	command -v cargo >/dev/null 2>&1 || die "cargo not found; install Rust with rustup first"
	command -v rustup >/dev/null 2>&1 || die "rustup not found; install Rust with rustup first"
}

ensure_rust_target() {
	if ! rustup target list --installed 2>/dev/null | grep -q "^${TARGET}\$"; then
		log "adding Rust target: ${TARGET}"
		rustup target add "${TARGET}"
	fi
}

install_linux_deps() {
	local packages_raw sudo_cmd=()
	packages_raw="$(target_apt_packages "${TARGET}" || true)"
	[[ -n "${packages_raw}" ]] || die "unsupported target for automatic Linux dependency install: ${TARGET}"
	command -v apt-get >/dev/null 2>&1 || die "automatic dependency install currently supports Debian/Ubuntu only; rerun with --skip-deps after installing a target gcc manually"
	if [[ "${EUID}" -ne 0 ]]; then
		command -v sudo >/dev/null 2>&1 || die "sudo is required to install dependencies"
		sudo_cmd=(sudo)
	fi
	log "installing cross-build dependencies: ${packages_raw}"
	# shellcheck disable=SC2086
	"${sudo_cmd[@]}" apt-get update -qq
	# shellcheck disable=SC2086
	"${sudo_cmd[@]}" apt-get install -y -qq ${packages_raw}
}

install_macos_deps() {
	local formula
	formula="$(target_brew_formula "${TARGET}" || true)"
	[[ -n "${formula}" ]] || die "unsupported target for automatic macOS dependency install: ${TARGET}"
	if ! command -v brew >/dev/null 2>&1; then
		if [[ -x /opt/homebrew/bin/brew ]]; then
			eval "$(/opt/homebrew/bin/brew shellenv)"
		elif [[ -x /usr/local/bin/brew ]]; then
			eval "$(/usr/local/bin/brew shellenv)"
		fi
	fi
	command -v brew >/dev/null 2>&1 || die "Homebrew is required for macOS cross-build dependencies"
	brew tap messense/macos-cross-toolchains >/dev/null 2>&1 || true
	if ! brew list --versions "${formula}" >/dev/null 2>&1; then
		log "brew install ${formula}"
		brew install "${formula}"
	fi
	if ! brew list --versions llvm >/dev/null 2>&1; then
		log "brew install llvm"
		brew install llvm
	fi
}

ensure_cross_deps() {
	ensure_cargo
	ensure_rust_target
	if [[ "${INSTALL_DEPS}" == "0" ]]; then
		return 0
	fi
	case "$(detect_host_os || true)" in
	linux)
		install_linux_deps
		;;
	macos)
		install_macos_deps
		;;
	*)
		die "unsupported host OS for dependency install; rerun with --skip-deps after installing a target gcc manually"
		;;
	esac
}

detect_linker() {
	local candidate brew_prefix
	if command -v brew >/dev/null 2>&1; then
		brew_prefix="$(target_brew_formula "${TARGET}" 2>/dev/null | xargs -I{} brew --prefix {} 2>/dev/null || true)"
		if [[ -n "${brew_prefix}" && -d "${brew_prefix}/bin" ]]; then
			export PATH="${brew_prefix}/bin:${PATH}"
		fi
	fi
	while IFS= read -r candidate; do
		[[ -n "${candidate}" ]] || continue
		if command -v "${candidate}" >/dev/null 2>&1; then
			command -v "${candidate}"
			return 0
		fi
	done < <(linker_candidates "${TARGET}")
	return 1
}

setup_cross_env() {
	local linker env_key cc_key bin_dir bin_prefix gcc_include sysroot include_dir clang_triple extra_args
	linker="$(detect_linker)" || die "target linker not found for ${TARGET}; rerun without --skip-deps or install one of: $(linker_candidates "${TARGET}" | xargs)"
	env_key="$(target_env_key "${TARGET}")"
	cc_key="$(target_cc_key "${TARGET}")"
	clang_triple="$(target_clang_triple "${TARGET}")"

	export "CARGO_TARGET_${env_key}_LINKER=${linker}"
	export "CC_${cc_key}=${linker}"
	export PKG_CONFIG_ALLOW_CROSS=1

	bin_dir="$(dirname "${linker}")"
	bin_prefix="$(basename "${linker}")"
	bin_prefix="${bin_prefix%gcc}"
	if [[ -x "${bin_dir}/${bin_prefix}g++" ]]; then
		export "CXX_${cc_key}=${bin_dir}/${bin_prefix}g++"
	fi
	if [[ -x "${bin_dir}/${bin_prefix}ar" ]]; then
		export "AR_${cc_key}=${bin_dir}/${bin_prefix}ar"
	fi

	gcc_include="$("${linker}" -print-file-name=include 2>/dev/null || true)"
	sysroot="$("${linker}" -print-sysroot 2>/dev/null || true)"
	include_dir=""
	for include_dir in \
		"${sysroot}/usr/include" \
		"${sysroot}/include" \
		"/usr/aarch64-linux-gnu/include" \
		"/usr/arm-linux-gnueabihf/include"; do
		if [[ -n "${include_dir}" && -d "${include_dir}" ]]; then
			break
		fi
	done

	if [[ -n "${gcc_include}" && -d "${gcc_include}" ]]; then
		extra_args="--target=${clang_triple} -I${gcc_include}"
		if [[ -n "${include_dir}" && -d "${include_dir}" ]]; then
			extra_args="${extra_args} -I${include_dir}"
		fi
		export "BINDGEN_EXTRA_CLANG_ARGS_${cc_key}=${extra_args}"
	fi

	log "cross environment ready: target=${TARGET}, linker=${linker}"
	log "output directory: $(target_release_dir "${SCRIPT_DIR}" "${TARGET}")"
}

parse_args() {
	while [[ $# -gt 0 ]]; do
		case "$1" in
		--target)
			[[ $# -ge 2 ]] || die "--target requires an argument"
			TARGET="$2"
			shift 2
			;;
		--workspace)
			BUILD_SCOPE="workspace"
			PACKAGE_NAME=""
			BIN_NAME=""
			shift
			;;
		--package|-p)
			[[ $# -ge 2 ]] || die "--package requires an argument"
			BUILD_SCOPE="package"
			PACKAGE_NAME="$2"
			BIN_NAME=""
			shift 2
			;;
		--bin)
			[[ $# -ge 2 ]] || die "--bin requires an argument"
			BUILD_SCOPE="bin"
			BIN_NAME="$2"
			PACKAGE_NAME=""
			shift 2
			;;
		--clean)
			DO_CLEAN=1
			shift
			;;
		--with-ui)
			SKIP_UI=0
			shift
			;;
		--no-ui)
			SKIP_UI=1
			shift
			;;
		--skip-deps)
			INSTALL_DEPS=0
			shift
			;;
		--sync-release-bin)
			SYNC_RELEASE_BIN=1
			shift
			;;
		-h|--help)
			usage
			exit 0
			;;
		--)
			shift
			CARGO_EXTRA_ARGS=("$@")
			break
			;;
		*)
			die "unknown argument: $1"
			;;
		esac
	done
}

run_build() {
	if [[ "${DO_CLEAN}" == "1" ]]; then
		log "cleaning cargo artifacts"
		cargo clean
	fi

	case "${BUILD_SCOPE}" in
	workspace)
		local -a args=(--target "${TARGET}")
		if [[ "${SKIP_UI}" == "1" ]]; then
			args=(no-ui "${args[@]}")
		fi
		if [[ "${#CARGO_EXTRA_ARGS[@]}" -gt 0 ]]; then
			die "extra cargo args are only supported with --package or --bin"
		fi
		log "building workspace via build-all.sh"
		bash "${SCRIPT_DIR}/build-all.sh" "${args[@]}"
		;;
	package)
		log "syncing skill docs"
		python3 "${SCRIPT_DIR}/scripts/sync_skill_docs.py"
		log "building package: ${PACKAGE_NAME}"
		cargo build --release --target "${TARGET}" -p "${PACKAGE_NAME}" "${CARGO_EXTRA_ARGS[@]}"
		;;
	bin)
		log "syncing skill docs"
		python3 "${SCRIPT_DIR}/scripts/sync_skill_docs.py"
		log "building binary: ${BIN_NAME}"
		cargo build --release --target "${TARGET}" --bin "${BIN_NAME}" "${CARGO_EXTRA_ARGS[@]}"
		;;
	*)
		die "unknown build scope: ${BUILD_SCOPE}"
		;;
	esac
}

main() {
	parse_args "$@"
	TARGET="$(normalize_target "${TARGET}")"
	ensure_cross_deps
	setup_cross_env
	run_build

	if [[ "${SYNC_RELEASE_BIN}" == "1" ]]; then
		log "syncing executables to release-bin"
		bash "${SCRIPT_DIR}/scripts/sync-release-bin.sh" "$(target_release_dir "${SCRIPT_DIR}" "${TARGET}")"
	fi

	log "cross build completed: $(target_release_dir "${SCRIPT_DIR}" "${TARGET}")"
}

main "$@"
