#!/usr/bin/env bash
# zh: 构建整个 RustClaw 工作区；运行时提示保持英文，中文说明仅作为维护注释。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"

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
		/usr/local/opt/llvm/lib; do
		if [[ -e "$candidate/libclang.so" ]]; then
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
		/usr/local/opt/llvm/lib; do
		if compgen -G "$candidate/libclang*.so*" >/dev/null 2>&1; then
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

if [[ -f "$HOME/.cargo/env" ]]; then
	. "$HOME/.cargo/env"
fi
ensure_cargo
ensure_protoc
ensure_bindgen_toolchain

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

echo "Syncing skill docs (INTERFACE.md + prompts/layers/generated/skills/*.md)..."
python3 "$SCRIPT_DIR/scripts/sync_skill_docs.py"

BUILD_PROFILE="release"
DO_CLEAN=0
REQUESTED_TARGET="host"
EXTRA_TARGETS=()

# Release-only build; keep compatibility with legacy `release` arguments.
# Use SKIP_UI=1 or `no-ui` to skip the UI build.
SKIP_UI="${SKIP_UI:-0}"
for arg in "$@"; do
	case "$arg" in
	release)
		;;
	clean)
		DO_CLEAN=1
		;;
	no-ui)
		SKIP_UI=1
		;;
	*)
		:
		;;
	esac
done

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
	--target)
		REQUESTED_TARGET="${2:?Missing argument for --target}"
		shift 2
		;;
	--extra-target)
		EXTRA_TARGETS+=("${2:?Missing argument for --extra-target}")
		shift 2
		;;
	-h|--help)
		echo "Usage: ./build-all.sh [release] [clean] [no-ui] [--target host|<triple>] [--extra-target <triple>]"
		echo "  host build: output to target/release"
		echo "  cross build: output to target/<triple>/release"
		exit 0
		;;
	*)
		echo "Usage: ./build-all.sh [release] [clean] [no-ui] [--target host|<triple>] [--extra-target <triple>]"
		echo "  host build: output to target/release"
		echo "  cross build: output to target/<triple>/release"
		exit 1
		;;
	esac
done

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
for extra_target in "${EXTRA_TARGETS[@]}"; do
	append_unique_target "$(resolve_requested_target "$extra_target")"
done

if [[ -d "$SCRIPT_DIR/UI" ]] && [[ "$SKIP_UI" != "1" ]]; then
	echo "Building and deploying UI assets to nginx..."
	bash "$SCRIPT_DIR/build-ui-nginx.sh"
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
configure_cargo_build_jobs_for_small_host
if [[ "${#TARGETS_TO_BUILD[@]}" -gt 1 ]]; then
	echo "Extra targets: ${TARGETS_TO_BUILD[*]:1}"
fi

# Ensure runtime binaries exist for deployment/start scripts. Skill Store
# entries marked `install_mode = "on_demand"` stay in the workspace for normal
# development and tests, but release builds must not compile them proactively.
WORKSPACE_METADATA="$(cargo metadata --no-deps --format-version 1)"
export RUSTCLAW_WORKSPACE_METADATA="$WORKSPACE_METADATA"

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
RUSTCLAW_ON_DEMAND_PACKAGES="$(printf '%s\n' "${ON_DEMAND_PACKAGES[@]:-}")"
export RUSTCLAW_ON_DEMAND_PACKAGES

REQUIRED_BINS=()
while IFS= read -r bin; do
	[[ -n "$bin" ]] && REQUIRED_BINS+=("$bin")
done < <(
	python3 - <<'PY'
import json
import os
import sys

raw = os.environ.get("RUSTCLAW_WORKSPACE_METADATA", "").strip()
if not raw:
    raise SystemExit(1)
data = json.loads(raw)
workspace_members = set(data.get("workspace_members", []))
on_demand_packages = {
    value.strip()
    for value in os.environ.get("RUSTCLAW_ON_DEMAND_PACKAGES", "").splitlines()
    if value.strip()
}
bins = set()

for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    if pkg.get("name") in on_demand_packages:
        continue
    for target in pkg.get("targets", []):
        kinds = target.get("kind", [])
        if "bin" in kinds:
            name = (target.get("name") or "").strip()
            if name:
                bins.add(name)

for name in sorted(bins):
    print(name)
PY
)

if [[ "${#REQUIRED_BINS[@]}" -eq 0 ]]; then
	echo "No workspace binary targets discovered via cargo metadata."
	exit 1
fi

for target in "${TARGETS_TO_BUILD[@]}"; do
	if [[ "$target" != "$HOST_TARGET" ]] && command -v rustup >/dev/null 2>&1; then
		rustup target add "$target" >/dev/null 2>&1 || true
	fi
	echo "Building target: $target"
	CARGO_WORKSPACE_ARGS=(--workspace --release)
	for package in "${ON_DEMAND_PACKAGES[@]:-}"; do
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
done

echo "Build completed."
echo "Primary output: $(preferred_release_dir_for_target "$SCRIPT_DIR" "$PRIMARY_TARGET")"
echo "Verified binaries: ${REQUIRED_BINS[*]}"
