#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SCRIPT_DIR"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/shell_compat.sh"
configure_platform_command_path
configure_python3_with_tomllib

TARGET="${1:-host}"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
if [[ -z "$HOST_TARGET" ]]; then
	echo "Unable to determine the host Rust target." >&2
	exit 1
fi
if [[ "$TARGET" == "host" ]]; then
	TARGET="$HOST_TARGET"
fi

PACKAGES=()
while IFS= read -r package; do
	[[ -n "$package" ]] && PACKAGES+=("$package")
done < <(
	python3 "$SCRIPT_DIR/scripts/skill_store_packages.py" \
		--scope platform-precompiled --target "$TARGET" --format packages
)

if [[ "${#PACKAGES[@]}" -eq 0 ]]; then
	echo "No platform-compatible Skill Store Cargo packages for target=$TARGET."
	exit 0
fi

configure_cargo_build_environment
CARGO_ARGS=(build --release --locked)
if [[ "$TARGET" != "$HOST_TARGET" ]]; then
	CARGO_ARGS+=(--target "$TARGET")
fi
for package in "${PACKAGES[@]}"; do
	CARGO_ARGS+=(-p "$package")
done

echo "Precompiling Skill Store packages for target=$TARGET: ${PACKAGES[*]}"
cargo "${CARGO_ARGS[@]}"

if [[ "$TARGET" == "$HOST_TARGET" ]]; then
	BINARY_DIR="$SCRIPT_DIR/target/release"
else
	BINARY_DIR="$SCRIPT_DIR/target/$TARGET/release"
fi
SDK_CLI="$SCRIPT_DIR/target/release/skillctl"
if [[ ! -x "$SDK_CLI" ]]; then
	echo "Building host receipt verification helper..."
	cargo build --release --locked -p agent-skill-sdk --bin skillctl
fi

PACKAGE_ROOT="$SCRIPT_DIR/target/prebuilt-skill-packages/$TARGET"
python3 "$SCRIPT_DIR/scripts/project_skill_receipts.py" \
	--scope platform-precompiled \
	--target "$TARGET" \
	--binary-dir "$BINARY_DIR" \
	--sdk-cli "$SDK_CLI" \
	--package-root "$PACKAGE_ROOT"

while IFS= read -r skill_name; do
	[[ -n "$skill_name" ]] || continue
	"$SDK_CLI" receipt-verify "$PACKAGE_ROOT" "$skill_name" >/dev/null
done < <(
	python3 "$SCRIPT_DIR/scripts/skill_store_packages.py" \
		--scope platform-precompiled --target "$TARGET" --format skills
)

echo "Platform Skill Store precompiles ready: $PACKAGE_ROOT"
