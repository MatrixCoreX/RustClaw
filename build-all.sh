#!/usr/bin/env bash
# zh: 构建整个 agent runtime 工作区；运行时提示保持英文，中文说明仅作为维护注释。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
configure_platform_command_path
configure_python3_with_tomllib

# ----- Ensure Cargo (Rust) is installed -----
# zh: 确保本机已有 Rust/Cargo；缺失时尝试自动安装 rustup。
ensure_cargo() {
	if ! command -v cargo >/dev/null 2>&1; then
		echo "cargo not found. Installing Rust toolchain (rustup)..."
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	fi
	if [[ -f "$HOME/.cargo/env" ]]; then
		. "$HOME/.cargo/env"
	fi

	if command -v cargo >/dev/null 2>&1 && cargo --version >/dev/null 2>&1; then
		return 0
	fi

	if command -v rustup >/dev/null 2>&1; then
		echo "Cargo is installed, but no default Rust toolchain is configured. Installing/selecting stable..."
		rustup default stable
	fi

	if ! command -v cargo >/dev/null 2>&1 || ! cargo --version >/dev/null 2>&1; then
		echo "Rust install failed or cargo not in PATH. Please run: source \"\$HOME/.cargo/env\""
		exit 1
	fi
	echo "Rust toolchain ready."
}

# zh: 确保 protobuf 编译器可用，供依赖生成代码。
ensure_protoc() {
	if command -v protoc >/dev/null 2>&1; then
		export PROTOC
		PROTOC="$(command -v protoc)"
		return 0
	fi
	echo "protoc not found. Attempting to install Protocol Buffers compiler..."
	if command -v brew >/dev/null 2>&1; then
		brew install protobuf
	elif command -v apt-get >/dev/null 2>&1; then
		sudo apt-get update -qq && sudo apt-get install -y protobuf-compiler
	elif command -v dnf >/dev/null 2>&1; then
		sudo dnf install -y protobuf-compiler
	elif command -v yum >/dev/null 2>&1; then
		sudo yum install -y protobuf-compiler
	elif command -v zypper >/dev/null 2>&1; then
		sudo zypper --non-interactive install protobuf
	elif command -v pacman >/dev/null 2>&1; then
		sudo pacman -Sy --noconfirm protobuf
	elif command -v apk >/dev/null 2>&1; then
		sudo apk add protobuf
	else
		echo "Please install protoc first."
		echo "Debian/Ubuntu: sudo apt-get install protobuf-compiler"
		echo "macOS: brew install protobuf"
		exit 1
	fi
	if ! command -v protoc >/dev/null 2>&1; then
		echo "protoc still not found after install attempt."
		exit 1
	fi
	export PROTOC
	PROTOC="$(command -v protoc)"
	echo "protoc ready: $PROTOC"
}

# Detect libclang presence via ldconfig or common install paths.
# Accepts versioned names like libclang-20.so(.20) on Debian/Ubuntu.
detect_libclang_dir() {
	if [[ -n "${LIBCLANG_PATH:-}" && -d "${LIBCLANG_PATH}" ]]; then
		printf '%s\n' "${LIBCLANG_PATH}"
		return 0
	fi

	# Prefer directories that contain the unversioned libclang.so symlink
	# (bindgen/clang-sys is happiest with that). Fall back to dirs that only
	# have versioned names like libclang-20.so.
	local candidate
	for candidate in \
		/usr/lib/llvm-*/lib \
		/usr/lib/x86_64-linux-gnu \
		/usr/lib/aarch64-linux-gnu \
		/usr/lib64 \
		/usr/local/lib \
		/opt/homebrew/opt/llvm/lib \
		/usr/local/opt/llvm/lib \
		/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib \
		/Library/Developer/CommandLineTools/usr/lib; do
		if [[ -e "$candidate/libclang.so" || -e "$candidate/libclang.dylib" ]]; then
			printf '%s\n' "$candidate"
			return 0
		fi
	done

	local line
	line="$(ldconfig -p 2>/dev/null | grep -E 'libclang(-[0-9]+)?\.so' | head -n1 || true)"
	if [[ -n "$line" ]]; then
		local path="${line##*=> }"
		if [[ -n "$path" ]]; then
			local dir
			dir="$(dirname "$path")"
			if [[ -d "$dir" ]]; then
				printf '%s\n' "$dir"
				return 0
			fi
		fi
	fi

	for candidate in \
		/usr/lib/llvm-*/lib \
		/usr/lib/x86_64-linux-gnu \
		/usr/lib/aarch64-linux-gnu \
		/usr/lib64 \
		/usr/local/lib \
		/opt/homebrew/opt/llvm/lib \
		/usr/local/opt/llvm/lib \
		/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib \
		/Library/Developer/CommandLineTools/usr/lib; do
		if compgen -G "$candidate/libclang*.so*" >/dev/null 2>&1 ||
			compgen -G "$candidate/libclang*.dylib" >/dev/null 2>&1; then
			printf '%s\n' "$candidate"
			return 0
		fi
	done

	return 1
}

# zh: 确保 bindgen 所需 clang/libclang 可用。
ensure_bindgen_toolchain() {
	local libclang_dir=""
	local need_install=0
	if ! command -v clang >/dev/null 2>&1; then
		need_install=1
	else
		libclang_dir="$(detect_libclang_dir || true)"
		if [[ -z "$libclang_dir" ]]; then
			need_install=1
		fi
	fi

	if [[ "$need_install" == "1" ]]; then
		echo "clang/libclang not found. Attempting to install bindgen toolchain..."
		if command -v brew >/dev/null 2>&1; then
			brew install llvm
			if [[ -z "${LIBCLANG_PATH:-}" ]]; then
				local llvm_prefix=""
				llvm_prefix="$(brew --prefix llvm 2>/dev/null || true)"
				if [[ -n "$llvm_prefix" && -d "$llvm_prefix/lib" ]]; then
					export LIBCLANG_PATH="$llvm_prefix/lib"
				fi
			fi
		elif command -v apt-get >/dev/null 2>&1; then
			sudo apt-get update -qq && sudo apt-get install -y clang libclang-dev
		elif command -v dnf >/dev/null 2>&1; then
			sudo dnf install -y clang llvm-devel libclang
		elif command -v yum >/dev/null 2>&1; then
			sudo yum install -y clang llvm-devel libclang
		elif command -v zypper >/dev/null 2>&1; then
			sudo zypper --non-interactive install clang llvm-devel libclang
		elif command -v pacman >/dev/null 2>&1; then
			sudo pacman -Sy --noconfirm clang llvm
		elif command -v apk >/dev/null 2>&1; then
			sudo apk add clang llvm-dev libclang
		else
			echo "Please install clang and libclang first."
			echo "Debian/Ubuntu: sudo apt-get install clang libclang-dev"
			echo "macOS: brew install llvm"
			exit 1
		fi

		if ! command -v clang >/dev/null 2>&1; then
			echo "clang still not found after install attempt."
			exit 1
		fi
		libclang_dir="$(detect_libclang_dir || true)"
		if [[ -z "$libclang_dir" ]]; then
			echo "libclang still not found after install attempt."
			echo "You may need to set LIBCLANG_PATH manually."
			exit 1
		fi
	fi

	if [[ -z "${LIBCLANG_PATH:-}" && -n "$libclang_dir" ]]; then
		export LIBCLANG_PATH="$libclang_dir"
	fi
	echo "bindgen toolchain ready (LIBCLANG_PATH=${LIBCLANG_PATH:-auto})."
}

# ----- Ensure npm is installed (only needed when UI exists) -----
# zh: 仅在需要构建 UI 时检查 npm。
ensure_npm() {
	if command -v npm >/dev/null 2>&1; then
		return 0
	fi
	echo "npm not found. Attempting to install Node.js/npm..."
	if [[ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]]; then
		. "${NVM_DIR:-$HOME/.nvm}/nvm.sh"
		nvm install --lts
		nvm use --lts
	elif command -v brew >/dev/null 2>&1; then
		brew install node
	elif command -v apt-get >/dev/null 2>&1; then
		sudo apt-get update -qq && sudo apt-get install -y nodejs npm
	elif command -v dnf >/dev/null 2>&1; then
		sudo dnf install -y nodejs npm
	elif command -v yum >/dev/null 2>&1; then
		sudo yum install -y nodejs npm
	else
		echo "Please install Node.js and npm first."
		echo "macOS: brew install node"
		echo "Other systems: https://nodejs.org or: curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash"
		exit 1
	fi
	if ! command -v npm >/dev/null 2>&1; then
		echo "npm still not found after install attempt."
		exit 1
	fi
	echo "Node.js/npm ready."
}

BUILD_PROFILE="release"
DO_CLEAN=0
REQUESTED_TARGET="host"
EXTRA_TARGETS=()
TOOLCHAIN_MODE="${APP_TOOLCHAIN_MODE:-ensure}"
PRESERVE_NGINX="${APP_PRESERVE_NGINX:-0}"

# Release-only build; keep compatibility with legacy `release` arguments.
# Use SKIP_UI=1 or `no-ui` to skip the UI build.
SKIP_UI="${SKIP_UI:-0}"

while [[ $# -gt 0 ]]; do
	case "$1" in
	release)
		shift
		;;
	clean)
		DO_CLEAN=1
		shift
		;;
	no-ui)
		SKIP_UI=1
		shift
		;;
	preserve-nginx)
		PRESERVE_NGINX=1
		shift
		;;
	--target)
		REQUESTED_TARGET="${2:?Missing argument for --target}"
		shift 2
		;;
	--extra-target)
		EXTRA_TARGETS+=("${2:?Missing argument for --extra-target}")
		shift 2
		;;
	--check-toolchains)
		TOOLCHAIN_MODE="check"
		shift
		;;
	--update-toolchains)
		TOOLCHAIN_MODE="update"
		shift
		;;
	-h|--help)
		echo "Usage: ./build-all.sh [release] [clean] [no-ui] [preserve-nginx] [--target host|<triple>] [--extra-target <triple>] [--check-toolchains|--update-toolchains]"
		echo "  host build: output to target/release"
		echo "  cross build: output to target/<triple>/release"
		echo "  preserve-nginx: build UI/dist but do not install, reload, copy, or modify nginx"
		echo "  --check-toolchains: inspect available compiler/tool updates without upgrading installed tools"
		echo "  --update-toolchains: update installed Rust, Clang, protoc, Node.js, and npm before building"
		exit 0
		;;
	*)
		echo "Usage: ./build-all.sh [release] [clean] [no-ui] [preserve-nginx] [--target host|<triple>] [--extra-target <triple>] [--check-toolchains|--update-toolchains]"
		echo "  host build: output to target/release"
		echo "  cross build: output to target/<triple>/release"
		exit 1
		;;
	esac
done

case "$TOOLCHAIN_MODE" in
	ensure|check|update)
		;;
	*)
		echo "Unsupported APP_TOOLCHAIN_MODE: $TOOLCHAIN_MODE (expected ensure, check, or update)."
		exit 1
		;;
esac

case "$PRESERVE_NGINX" in
	0|1) ;;
	*)
		echo "APP_PRESERVE_NGINX must be 0 or 1."
		exit 1
		;;
esac

if [[ -f "$HOME/.cargo/env" ]]; then
	. "$HOME/.cargo/env"
fi
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/build_toolchain_manager.sh"

INCLUDE_UI_TOOLCHAIN=0
if [[ -d "$SCRIPT_DIR/UI" ]] && [[ "$SKIP_UI" != "1" ]]; then
	INCLUDE_UI_TOOLCHAIN=1
fi

if [[ "$TOOLCHAIN_MODE" == "update" ]]; then
	agent_update_rust
	agent_update_package_toolchains "$INCLUDE_UI_TOOLCHAIN"
fi

ensure_cargo
ensure_protoc
ensure_bindgen_toolchain
if [[ "$INCLUDE_UI_TOOLCHAIN" == "1" ]]; then
	ensure_npm
fi

if [[ "$TOOLCHAIN_MODE" == "check" ]]; then
	agent_check_toolchain_updates "$INCLUDE_UI_TOOLCHAIN"
fi
agent_report_build_toolchains
agent_validate_build_toolchains "$INCLUDE_UI_TOOLCHAIN"

echo "Syncing skill docs (INTERFACE.md + prompts/layers/generated/skills/*.md)..."
python3 "$SCRIPT_DIR/scripts/sync_skill_docs.py"

PRIMARY_TARGET="$(resolve_requested_target "$REQUESTED_TARGET")"
HOST_OS="$(detect_host_os || true)"
HOST_ARCH="$(detect_host_arch || true)"
HOST_TARGET="$(host_rust_target 2>/dev/null || true)"
PACKAGE_FLAVOR="$(package_flavor_for_target "$PRIMARY_TARGET" 2>/dev/null || printf '%s' "$PRIMARY_TARGET")"

TARGETS_TO_BUILD=()
append_unique_target() {
	local candidate="$1"
	local existing
	for existing in "${TARGETS_TO_BUILD[@]:-}"; do
		[[ "$existing" == "$candidate" ]] && return 0
	done
	TARGETS_TO_BUILD+=("$candidate")
}

append_unique_target "$PRIMARY_TARGET"
for extra_target in "${EXTRA_TARGETS[@]:-}"; do
	append_unique_target "$(resolve_requested_target "$extra_target")"
done

UI_BUILT=0
if [[ -d "$SCRIPT_DIR/UI" ]] && [[ "$SKIP_UI" != "1" ]]; then
	echo "Building UI assets; deployment is deferred until the full build succeeds..."
	bash "$SCRIPT_DIR/build-ui-nginx.sh" --build
	UI_BUILT=1
elif [[ "$SKIP_UI" == "1" ]]; then
	echo "Skipping UI build (SKIP_UI=1 or no-ui)."
else
	echo "UI directory not found, skipping UI build."
fi

if [[ "$DO_CLEAN" == "1" ]]; then
	echo "Cleaning previous build artifacts..."
	cargo clean
fi

echo "Building runtime workspace with profile: $BUILD_PROFILE"
echo "Host platform: ${HOST_OS:-unknown}/${HOST_ARCH:-unknown}"
echo "Primary target: $PRIMARY_TARGET"
echo "Primary output: $(preferred_release_dir_for_target "$SCRIPT_DIR" "$PRIMARY_TARGET")"
echo "Flavor tag: $PACKAGE_FLAVOR"
configure_cargo_build_environment
if [[ "${#TARGETS_TO_BUILD[@]}" -gt 1 ]]; then
	echo "Extra targets: ${TARGETS_TO_BUILD[*]:1}"
fi

# Ensure runtime/core-tool binaries exist for deployment/start scripts. Runner
# skill packages are selected from the registry for each target OS. Skill Store
# entries marked `install_mode = "on_demand"` are never compiled proactively.
WORKSPACE_METADATA_FILE="$(mktemp)"
trap 'rm -f "$WORKSPACE_METADATA_FILE"' EXIT
cargo metadata --no-deps --format-version 1 >"$WORKSPACE_METADATA_FILE"

ON_DEMAND_PACKAGES=()
while IFS=$'\t' read -r package bin; do
	[[ -n "$package" ]] && ON_DEMAND_PACKAGES+=("$package")
	[[ -n "$bin" ]] || {
		echo "On-demand skill package is missing a runner binary: $package"
		exit 1
	}
done < <(
	python3 "$SCRIPT_DIR/scripts/skill_store_packages.py" --format pairs
)

if [[ "${#ON_DEMAND_PACKAGES[@]}" -gt 0 ]]; then
	echo "Skill Store packages excluded from proactive build: ${ON_DEMAND_PACKAGES[*]}"
fi

PRIMARY_REQUIRED_BINS=()
for target in "${TARGETS_TO_BUILD[@]}"; do
	if [[ "$target" != "$HOST_TARGET" ]] && command -v rustup >/dev/null 2>&1; then
		rustup target add "$target" >/dev/null 2>&1 || true
	fi
	echo "Building target: $target"
	BUILD_EXCLUDED_PACKAGES=()
	while IFS= read -r package; do
		[[ -n "$package" ]] && BUILD_EXCLUDED_PACKAGES+=("$package")
	done < <(
		python3 "$SCRIPT_DIR/scripts/skill_store_packages.py" \
			--scope build-excludes --target "$target" --format packages
	)
	UNSUPPORTED_PACKAGES=()
	while IFS= read -r package; do
		[[ -n "$package" ]] && UNSUPPORTED_PACKAGES+=("$package")
	done < <(
		python3 "$SCRIPT_DIR/scripts/skill_store_packages.py" \
			--scope unsupported-proactive --target "$target" --format packages
	)
	if [[ "${#UNSUPPORTED_PACKAGES[@]}" -gt 0 ]]; then
		echo "Runner packages unsupported for target=$target: ${UNSUPPORTED_PACKAGES[*]}"
	fi

	APP_BUILD_EXCLUDED_PACKAGES="$(printf '%s\n' "${BUILD_EXCLUDED_PACKAGES[@]:-}")"
	export APP_BUILD_EXCLUDED_PACKAGES
	REQUIRED_BINS=()
	while IFS= read -r bin; do
		[[ -n "$bin" ]] && REQUIRED_BINS+=("$bin")
	done < <(
		python3 - "$WORKSPACE_METADATA_FILE" <<'PY'
import json
import os
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    data = json.load(handle)
workspace_members = set(data.get("workspace_members", []))
excluded = {
    value.strip()
    for value in os.environ.get("APP_BUILD_EXCLUDED_PACKAGES", "").splitlines()
    if value.strip()
}
bins = {
    target.get("name", "").strip()
    for package in data.get("packages", [])
    if package.get("id") in workspace_members and package.get("name") not in excluded
    for target in package.get("targets", [])
    if "bin" in target.get("kind", []) and target.get("name", "").strip()
}
for name in sorted(bins):
    print(name)
PY
	)
	if [[ "${#REQUIRED_BINS[@]}" -eq 0 ]]; then
		echo "No workspace binary targets selected for target=$target."
		exit 1
	fi
	if [[ "$target" == "$PRIMARY_TARGET" ]]; then
		PRIMARY_REQUIRED_BINS=("${REQUIRED_BINS[@]}")
	fi

	CARGO_WORKSPACE_ARGS=(--workspace --release)
	for package in "${BUILD_EXCLUDED_PACKAGES[@]:-}"; do
		[[ -n "$package" ]] && CARGO_WORKSPACE_ARGS+=(--exclude "$package")
	done
	if [[ "$target" == "$HOST_TARGET" ]]; then
		cargo build "${CARGO_WORKSPACE_ARGS[@]}"
	else
		cargo build "${CARGO_WORKSPACE_ARGS[@]}" --target "$target"
	fi
	OUT_DIR="$(preferred_release_dir_for_target "$SCRIPT_DIR" "$target")"
	MISSING=0
	for bin in "${REQUIRED_BINS[@]}"; do
		if [[ ! -x "$OUT_DIR/$bin" ]]; then
			echo "Missing binary: $OUT_DIR/$bin"
			MISSING=1
		fi
	done
	if [[ "$MISSING" == "1" ]]; then
		echo "Build finished, but required binaries are missing for target=$target profile=$BUILD_PROFILE."
		echo "Try: cargo build ${CARGO_WORKSPACE_ARGS[*]} --target $target"
		exit 1
	fi
	if [[ "$target" == "$HOST_TARGET" ]]; then
		echo "Projecting proactive skill binaries into verified receipts for $target..."
		python3 "$SCRIPT_DIR/scripts/project_skill_receipts.py" \
			--target "$target" \
			--binary-dir "$OUT_DIR" \
			--sdk-cli "$OUT_DIR/skillctl" \
			--package-root "$SCRIPT_DIR/target/skill-packages/$target"
		echo "Activating verified proactive skill receipts for the local runtime..."
		python3 "$SCRIPT_DIR/scripts/project_skill_receipts.py" \
			--target host \
			--binary-dir "$OUT_DIR" \
			--sdk-cli "$OUT_DIR/skillctl" \
			--package-root "$SCRIPT_DIR/data/skill-packages"
	fi
done

if [[ "$UI_BUILT" == "1" ]]; then
	if [[ "$PRESERVE_NGINX" == "1" ]]; then
		echo "Preserving nginx as requested; UI/dist was built but nginx was not modified."
	elif [[ "$PRIMARY_TARGET" == "$HOST_TARGET" ]]; then
		echo "Checking whether an existing agent UI nginx site needs the latest UI..."
		bash "$SCRIPT_DIR/build-ui-nginx.sh" --copy-if-configured
	else
		echo "Skipping nginx UI deployment for cross-target build: $PRIMARY_TARGET"
	fi
fi

echo "Build completed."
echo "Primary output: $(preferred_release_dir_for_target "$SCRIPT_DIR" "$PRIMARY_TARGET")"
echo "Verified binaries: ${PRIMARY_REQUIRED_BINS[*]}"
