#!/usr/bin/env bash
set -euo pipefail

RUSTUP_INIT_VERSION="1.28.2"
os="$(uname -s)"
arch="$(uname -m)"
case "$os:$arch" in
  Linux:x86_64|Linux:amd64)
    triple="x86_64-unknown-linux-gnu"
    expected_sha256="20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c"
    ;;
  Linux:aarch64|Linux:arm64)
    triple="aarch64-unknown-linux-gnu"
    expected_sha256="e3853c5a252fca15252d07cb23a1bdd9377a8c6f3efa01531109281ae47f841c"
    ;;
  Darwin:x86_64)
    triple="x86_64-apple-darwin"
    expected_sha256="9c331076f62b4d0edeae63d9d1c9442d5fe39b37b05025ec8d41c5ed35486496"
    ;;
  Darwin:arm64|Darwin:aarch64)
    triple="aarch64-apple-darwin"
    expected_sha256="20ef5516c31b1ac2290084199ba77dbbcaa1406c45c1d978ca68558ef5964ef5"
    ;;
  *)
    echo "Unsupported rustup bootstrap platform: $os/$arch" >&2
    exit 1
    ;;
esac

command -v curl >/dev/null 2>&1 || {
  echo "curl is required to download the pinned Rust installer." >&2
  exit 1
}

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
installer="$tmp_dir/rustup-init"
url="https://static.rust-lang.org/rustup/archive/${RUSTUP_INIT_VERSION}/${triple}/rustup-init"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
  --output "$installer" "$url"

actual_sha256="$(python3 - "$installer" <<'PY'
import hashlib
from pathlib import Path
import sys

print(hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)"
if [[ "$actual_sha256" != "$expected_sha256" ]]; then
  echo "Pinned rustup installer checksum mismatch." >&2
  exit 1
fi
chmod 700 "$installer"
"$installer" -y "$@"
