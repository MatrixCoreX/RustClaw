#!/usr/bin/env bash
set -euo pipefail

# Rustup ships an LLD matching rustc. Reusing it avoids a separate system lld
# dependency and substantially reduces memory/time for RustClaw's large test
# and runtime links. A caller can opt out when diagnosing toolchain issues.
if [[ "${RUSTCLAW_DISABLE_BUNDLED_LLD:-0}" != "1" ]] \
  && command -v rustc >/dev/null 2>&1 \
  && command -v clang >/dev/null 2>&1; then
  rust_host="$(rustc -vV | awk '/^host:/ {print $2; exit}')"
  rust_sysroot="$(rustc --print sysroot)"
  bundled_lld="${rust_sysroot}/lib/rustlib/${rust_host}/bin/gcc-ld/ld.lld"
  if [[ -x "$bundled_lld" ]]; then
    exec clang "-fuse-ld=${bundled_lld}" "$@"
  fi
  if command -v ld.lld >/dev/null 2>&1; then
    exec clang -fuse-ld=lld "$@"
  fi
fi

exec cc "$@"
