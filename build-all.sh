#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"

# ----- Ensure Cargo (Rust) is installed -----
ensure_cargo() {
	if command -v cargo >/dev/null 2>&1; then
		return 0
	fi
	echo "cargo not found. Installing Rust toolchain (rustup)..."
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	if [[ -f "$HOME/.cargo/env" ]]; then
		. "$HOME/.cargo/env"
	fi
	if ! command -v cargo >/dev/null 2>&1; then
		echo "Rust install failed or cargo not in PATH. Please run: source \"\$HOME/.cargo/env\""
		exit 1
	fi
	echo "Rust toolchain installed."
}

if [[ -f "$HOME/.cargo/env" ]]; then
	. "$HOME/.cargo/env"
fi
ensure_cargo

# ----- Ensure npm is installed (only needed when UI exists) -----
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
	NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=512}" bash "$SCRIPT_DIR/build-ui-nginx.sh"
elif [[ "$SKIP_UI" == "1" ]]; then
	echo "Skipping UI build (SKIP_UI=1 or no-ui)."
else
	echo "UI directory not found, skipping UI build."
fi

if [[ "$DO_CLEAN" == "1" ]]; then
	echo "Cleaning previous build artifacts..."
	cargo clean
fi

echo "Building workspace with profile: $BUILD_PROFILE"
echo "Host platform: ${HOST_OS:-unknown}/${HOST_ARCH:-unknown}"
echo "Primary target: $PRIMARY_TARGET"
echo "Primary output: $(preferred_release_dir_for_target "$SCRIPT_DIR" "$PRIMARY_TARGET")"
echo "Flavor tag: $PACKAGE_FLAVOR"
if [[ "${#TARGETS_TO_BUILD[@]}" -gt 1 ]]; then
	echo "Extra targets: ${TARGETS_TO_BUILD[*]:1}"
fi

# Ensure runtime binaries exist for deployment/start scripts.
# Auto-discover all workspace bin targets to avoid missing newly added skills.
WORKSPACE_METADATA="$(cargo metadata --no-deps --format-version 1)"
export RUSTCLAW_WORKSPACE_METADATA="$WORKSPACE_METADATA"

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
bins = set()

for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
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
	if [[ "$target" == "$HOST_TARGET" ]]; then
		cargo build --workspace --release
	else
		cargo build --workspace --release --target "$target"
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
		echo "Try: cargo build --workspace --release --target $target"
		exit 1
	fi
done

echo "Build completed."
echo "Primary output: $(preferred_release_dir_for_target "$SCRIPT_DIR" "$PRIMARY_TARGET")"
echo "Verified binaries: ${REQUIRED_BINS[*]}"
