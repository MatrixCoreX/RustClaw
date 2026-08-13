#!/usr/bin/env bash
set -euo pipefail

linker_target=""
for linker_arg in "$@"; do
  case "$linker_arg" in
    */target/aarch64-unknown-linux-gnu/*|*/rustlib/aarch64-unknown-linux-gnu/*)
      linker_target="aarch64-unknown-linux-gnu"
      break
      ;;
    */target/armv7-unknown-linux-gnueabihf/*|*/rustlib/armv7-unknown-linux-gnueabihf/*)
      linker_target="armv7-unknown-linux-gnueabihf"
      break
      ;;
  esac
done

case "$linker_target" in
  aarch64-unknown-linux-gnu)
    for cross_linker in aarch64-linux-gnu-gcc aarch64-unknown-linux-gnu-gcc; do
      if command -v "$cross_linker" >/dev/null 2>&1; then
        exec "$cross_linker" "$@"
      fi
    done
    echo "A Linux aarch64 link was requested, but no aarch64 cross linker is available." >&2
    exit 127
    ;;
  armv7-unknown-linux-gnueabihf)
    for cross_linker in arm-linux-gnueabihf-gcc armv7-unknown-linux-gnueabihf-gcc; do
      if command -v "$cross_linker" >/dev/null 2>&1; then
        exec "$cross_linker" "$@"
      fi
    done
    echo "A Linux armv7 link was requested, but no armv7 cross linker is available." >&2
    exit 127
    ;;
esac

# Rustup ships an LLD matching rustc. Reusing it avoids a separate system lld
# dependency and substantially reduces memory/time for the runtime's large test
# and runtime links. A caller can opt out when diagnosing toolchain issues.
if [[ "${APP_DISABLE_BUNDLED_LLD:-0}" != "1" ]] \
  && command -v rustc >/dev/null 2>&1 \
  && command -v clang >/dev/null 2>&1; then
  rust_host="$(rustc -vV | sed -n 's/^host: //p')"
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
